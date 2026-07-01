#!/usr/bin/env node
/**
 * workflow-todo-write.js — Harness Workflow TodoWrite 计算 + 运行日志追加
 *
 * 在 state.json 写入后调用，完成两项工作：
 * 1. 追加运行日志到 {module}-run-log.md（文件副作用，不回传 Agent）
 * 2. 计算 TodoWrite todos[] 数组，纯 JSON 输出到 stdout（仅主 state 触发，回传 Agent）
 *
 * 返回值约定：
 * - stdout 只输出纯 todos[] JSON 数组，无提示语、无 markdown 围栏、无 [LOG]/[TODO] 前缀
 * - per-plan state 写入：stdout 为空（不刷主 Agent todo）
 * - 主 state 写入：stdout = `[ {content, status, priority}, ... ]`
 * - 错误信息走 stderr，不污染 stdout
 *
 * workflow.yaml 驱动：
 * - stage 顺序、on_failure 跳转、skill/agent 均从 workflow.yaml 读取
 * - 名词→动名词映射表内置（允许硬编码，属命名约定）
 * - 每 plan 在 todo 中只占 1 项（当前活跃 stage 的动作），不做全景投影
 *
 * Exit codes: 0=成功，1=错误
 * Usage: node workflow-todo-write.js <state-json-path>
 */
const fs = require("fs");
const path = require("path");

// ── 名词→动名词映射表（允许硬编码，属命名约定）──────────────────
// yaml stage name（名词）→ state.json stage value（动名词）
const NOUN_TO_STATE = {
  code: "coding",
  review: "reviewing",
  evaluate: "evaluating",
  fix: "fixing",
};

// 名词→中文显示名（用于 todo content）
const NOUN_TO_DISPLAY = {
  code: "编码",
  review: "检视",
  evaluate: "评估",
  fix: "修复",
};

// trigger_stage → 显示名 + 最大重试轮次（允许硬编码，retry 保持硬编码）
const TRIGGER_INFO = {
  reviewing: { display: "检视", maxRounds: 3 },
  evaluating: { display: "评估", maxRounds: 5 },
};

// ── 最小 YAML 解析器（零外部依赖，仅支持 workflow.yaml 的 schema 子集）──
// 支持：注释、key: value、key: 后跟列表（- item）、key: 后跟嵌套对象
function parseSimpleYaml(text) {
  const lines = text.split(/\r?\n/).map((raw) => {
    let content = raw;
    const hashIdx = content.indexOf("#");
    if (hashIdx >= 0) content = content.slice(0, hashIdx);
    const indent = content.match(/^(\s*)/)[1].length;
    return { indent, content: content.trim(), raw };
  }).filter((l) => l.content !== "");

  let pos = 0;

  function parseScalar(val) {
    val = val.trim();
    if (/^\d+$/.test(val)) return parseInt(val, 10);
    if (val === "true") return true;
    if (val === "false") return false;
    return val;
  }

  function splitKV(str) {
    const idx = str.indexOf(":");
    if (idx < 0) return null;
    return [str.slice(0, idx).trim(), str.slice(idx + 1).trim()];
  }

  function parseBlock(parentIndent) {
    const result = {};
    while (pos < lines.length) {
      const line = lines[pos];
      if (line.indent <= parentIndent) break;

      const kv = splitKV(line.content);
      if (!kv) { pos++; continue; }

      const [key, val] = kv;
      if (val !== "") {
        result[key] = parseScalar(val);
        pos++;
      } else {
        pos++;
        if (pos < lines.length && lines[pos].indent > line.indent) {
          if (lines[pos].content.startsWith("- ")) {
            result[key] = parseList(line.indent);
          } else {
            result[key] = parseBlock(line.indent);
          }
        } else {
          result[key] = null;
        }
      }
    }
    return result;
  }

  function parseList(parentIndent) {
    const arr = [];
    while (pos < lines.length) {
      const line = lines[pos];
      if (line.indent <= parentIndent) break;
      if (!line.content.startsWith("- ")) break;

      const itemContent = line.content.slice(2);
      const item = {};
      const kv = splitKV(itemContent);
      if (kv && kv[1] !== "") {
        item[kv[0]] = parseScalar(kv[1]);
      }

      pos++;

      while (pos < lines.length) {
        const sub = lines[pos];
        if (sub.indent <= line.indent) break;
        if (sub.content.startsWith("- ")) break;
        const subKV = splitKV(sub.content);
        if (subKV) {
          item[subKV[0]] = subKV[1] !== "" ? parseScalar(subKV[1]) : null;
        }
        pos++;
      }

      arr.push(item);
    }
    return arr;
  }

  return parseBlock(-1);
}

// ── Workflow Config 加载 ─────────────────────────────────────
function loadWorkflowConfig(harnessDir) {
  const yamlPath = path.join(harnessDir, "workflow.yaml");
  if (!fs.existsSync(yamlPath)) {
    process.stderr.write(`[workflow-todo-write] workflow.yaml not found: ${yamlPath}\n`);
    return null;
  }
  try {
    const text = fs.readFileSync(yamlPath, "utf-8");
    return parseSimpleYaml(text);
  } catch (exc) {
    process.stderr.write(`[workflow-todo-write] workflow.yaml parse error: ${exc}\n`);
    return null;
  }
}

// 返回 {type: "skill"|"agent", name: string} 或 null
function getStageDispatch(config, stageName) {
  for (const stage of config["local-stages"] || []) {
    if (stage.name === stageName) {
      if (stage.skill) return { type: "skill", name: stage.skill };
      if (stage.agent) return { type: "agent", name: stage.agent };
    }
  }
  for (const stage of config["global-stages"] || []) {
    if (stage.name === stageName) {
      if (stage.skill) return { type: "skill", name: stage.skill };
      if (stage.agent) return { type: "agent", name: stage.agent };
    }
  }
  const opt = config["optional-stages"] || {};
  if (opt[stageName]) {
    if (opt[stageName].skill) return { type: "skill", name: opt[stageName].skill };
    if (opt[stageName].agent) return { type: "agent", name: opt[stageName].agent };
  }
  return null;
}

// 根据 dispatch 信息生成 content 中的动作描述
// skill → "加载并执行 Skill xxx"
// agent → "派发 xxx sub-Agent"
function formatAction(dispatch) {
  if (!dispatch) return "";
  if (dispatch.type === "skill") return `加载并执行 Skill ${dispatch.name}`;
  if (dispatch.type === "agent") return `派发 ${dispatch.name} sub-Agent`;
  return dispatch.name || "";
}

function stateToNoun(stateValue) {
  for (const [noun, st] of Object.entries(NOUN_TO_STATE)) {
    if (st === stateValue) return noun;
  }
  return null;
}

// ── JSON 读取工具 ────────────────────────────────────────────
function readJsonSync(filePath) {
  let content = fs.readFileSync(filePath, "utf-8");
  if (content.charCodeAt(0) === 0xfeff || content.charCodeAt(0) === 0xfffe) {
    content = content.slice(1);
  }
  return JSON.parse(content);
}

function tryReadJson(filePath) {
  try {
    return readJsonSync(filePath);
  } catch {
    return null;
  }
}

function isMainState(statePath) {
  const base = path.basename(statePath);
  return base.endsWith("-workflow-state.json");
}

function inferModule(statePath) {
  let base = path.basename(statePath);
  if (base.endsWith("-workflow-state.json")) {
    return base.slice(0, -"-workflow-state.json".length);
  }
  if (base.endsWith("-state.json")) {
    base = base.slice(0, -"-state.json".length);
  }
  return base.includes("-") ? base.split("-")[0] : base;
}

// ── 运行日志 ─────────────────────────────────────────────────
function appendLog(state, statePath, logDir) {
  try {
    const truthSource = state.truth_source || "unknown";
    const moduleName = inferModule(statePath);
    const logFile = path.join(logDir, `${moduleName}-run-log.md`);

    fs.mkdirSync(logDir, { recursive: true });

    const now = new Date();
    const pad = (n) => String(n).padStart(2, "0");
    const timestamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`;

    const stage = state.stage || "unknown";
    const currentTask = state.current_task || "none";
    const completed = (state.tasks_completed || []).length;
    const remaining = (state.tasks_remaining || []).length;
    const fixing = state.fixing;
    const fixingStr = fixing
      ? `${fixing.trigger_stage} round ${fixing.round || 1}`
      : "none";
    const blockedStr = state.blocked_reason ? JSON.stringify(state.blocked_reason) : "none";

    const summary = `
## ${timestamp}
- Truth source: ${truthSource}
- Stage: ${stage}
- Current task: ${currentTask} (completed: ${completed}, remaining: ${remaining})
- Fixing: ${fixingStr}
- Blocked: ${blockedStr}
`;
    fs.appendFileSync(logFile, summary, "utf-8");

    return logFile;
  } catch (exc) {
    process.stderr.write(`[workflow-todo-write] 日志追加失败: ${exc}\n`);
    return null;
  }
}

function appendTodoCallLog(statePath, todos, logDir) {
  try {
    const moduleName = inferModule(statePath);
    const logFile = path.join(logDir, `${moduleName}-todo-call.log`);

    fs.mkdirSync(logDir, { recursive: true });

    const now = new Date();
    const pad = (n) => String(n).padStart(2, "0");
    const ts = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`;

    const iconMap = { completed: "v", in_progress: ">", pending: " " };
    const summary = todos
      .map((t, i) => {
        const icon = iconMap[t.status] || "?";
        return `${String(i).padStart(2, "0")}[${icon}]${t.content}`;
      })
      .join(" | ");

    const entry = JSON.stringify({
      ts,
      state: path.basename(statePath),
      todos: todos.length,
      summary,
    });
    fs.appendFileSync(logFile, entry + "\n", "utf-8");
  } catch (exc) {
    process.stderr.write(`[workflow-todo-write] todo 调用日志追加失败: ${exc}\n`);
  }
}

// ── 拓扑排序算 Wave 分组 ─────────────────────────────────────
function computeWaves(plans) {
  const planMap = new Map();
  for (const p of plans) {
    planMap.set(p.name, p.depends_on || []);
  }

  const waves = {};
  const planWave = {};
  let currentWave = 1;

  while (planMap.size > 0) {
    const ready = [];
    for (const [name, deps] of planMap) {
      const allDepsResolved = deps.every((d) => planWave.hasOwnProperty(d));
      if (allDepsResolved) {
        ready.push(name);
      }
    }

    if (ready.length === 0) {
      for (const name of planMap.keys()) {
        if (!waves[currentWave]) waves[currentWave] = [];
        waves[currentWave].push(name);
        planWave[name] = currentWave;
      }
      break;
    }

    waves[currentWave] = ready;
    for (const name of ready) {
      planWave[name] = currentWave;
      planMap.delete(name);
    }
    currentWave++;
  }

  return waves;
}

// ── 依赖检查 ─────────────────────────────────────────────────
function checkDepsSatisfied(dependsOn, planStatus) {
  if (!dependsOn || dependsOn.length === 0) return true;
  return dependsOn.every(
    (dep) => planStatus[dep] === "merged" || planStatus[dep] === "completed"
  );
}

// ── 构建 per-plan 的 todo 项（已完成 stage 各 1 项 + 当前 stage 1 项）─────
// 原则：已完成项 content 冻结不改，只改 status=completed。
// 初始每 plan 1 项 code [pending]，随推进增量增长（非全景投影）。
function buildPlanTodoItems(planName, planState, planFlowStatus, config, depsSatisfied, started) {
  const prefix = `${planName}: `;
  const localStages = config["local-stages"] || [];
  const items = [];

  function stageContent(noun, extraLabel) {
    const display = NOUN_TO_DISPLAY[noun] || noun;
    const dispatch = getStageDispatch(config, noun);
    const label = extraLabel || "";
    return `${prefix}${display} — ${formatAction(dispatch)}${label}`;
  }

  // 根据 stage 推导已完成的 stage nouns（按 local-stages 顺序）
  function getCompletedNouns() {
    if (planFlowStatus === "merged" || planFlowStatus === "completed" ||
        planState.stage === "completed" || planState.stage === "stage_completed") {
      return localStages.map((s) => s.name);
    }
    const stage = planState.stage || "coding";
    const fixing = planState.fixing;
    const triggerStage = fixing ? fixing.trigger_stage : null;
    if (stage === "coding") return [];
    if (stage === "reviewing") return ["code"];
    if (stage === "evaluating") return ["code", "review"];
    if (stage === "fixing" || stage === "blocked") {
      if (triggerStage === "reviewing") return ["code", "review"];
      if (triggerStage === "evaluating") return ["code", "review", "evaluate"];
      return [];
    }
    return [];
  }

  // 已完成项 [completed]（content 冻结）
  const completedNouns = getCompletedNouns();
  for (const noun of completedNouns) {
    items.push({
      content: stageContent(noun),
      status: "completed",
      priority: "high",
    });
  }

  // 终态：无活跃项
  if (planFlowStatus === "merged" || planFlowStatus === "completed" ||
      planState.stage === "completed" || planState.stage === "stage_completed") {
    return items;
  }

  // blocked → blocked 项 [in_progress]
  if (planFlowStatus === "blocked" || planState.stage === "blocked") {
    const fixing = planState.fixing;
    const triggerStage = fixing ? fixing.trigger_stage : null;
    let label = "🛑 blocked";
    if (triggerStage && TRIGGER_INFO[triggerStage]) {
      const ti = TRIGGER_INFO[triggerStage];
      const round = fixing ? (fixing.round || 1) : 1;
      label = `🛑 blocked（${ti.display}修复超限 r${round}/${ti.maxRounds}）`;
    }
    const display = triggerStage ? (TRIGGER_INFO[triggerStage]?.display || "编码") : "编码";
    items.push({
      content: `${prefix}${display} ${label}`,
      status: "in_progress",
      priority: "high",
    });
    return items;
  }

  // 未启动 → code [pending]
  if (!started) {
    items.push({
      content: stageContent("code"),
      status: "pending",
      priority: "high",
    });
    return items;
  }

  // 活跃 local stage → 当前项 [in_progress]
  const stage = planState.stage || "coding";
  if (stage === "fixing") {
    const fixing = planState.fixing;
    const triggerStage = fixing ? fixing.trigger_stage : null;
    let fixLabel = "";
    if (triggerStage && TRIGGER_INFO[triggerStage]) {
      const ti = TRIGGER_INFO[triggerStage];
      const round = fixing ? (fixing.round || 1) : 1;
      fixLabel = `（${ti.display}修复 r${round}/${ti.maxRounds}）`;
    }
    items.push({
      content: stageContent("fix", fixLabel),
      status: "in_progress",
      priority: "high",
    });
  } else {
    const noun = stateToNoun(stage);
    if (noun) {
      items.push({
        content: stageContent(noun),
        status: "in_progress",
        priority: "high",
      });
    }
  }

  return items;
}

// ── 构建 global-stages 的 todo 项 ────────────────────────────
// 已完成 [completed]（content 冻结）+ 当前 [in_progress] + 未完成 [pending]
function buildGlobalStageTodos(config, mainState) {
  const globalStages = config["global-stages"] || [];
  const todos = [];

  if (!globalStages.length) {
    return [{
      content: "Workflow 完成: Self-Check + 总结",
      status: "in_progress",
      priority: "medium",
    }];
  }

  function globalContent(st) {
    const display = NOUN_TO_DISPLAY[st.name] || st.name;
    const dispatch = st.skill ? { type: "skill", name: st.skill } : { type: "agent", name: st.agent };
    return `${display} — ${formatAction(dispatch)}`;
  }

  const currentStage = mainState.stage;

  if (currentStage === "completed") {
    for (const st of globalStages) {
      todos.push({
        content: globalContent(st),
        status: "completed",
        priority: "high",
      });
    }
    todos.push({
      content: "Workflow 完成: Self-Check + 总结",
      status: "in_progress",
      priority: "medium",
    });
    return todos;
  }

  // 找当前 global-stage 索引
  let currentIdx = -1;
  if (currentStage === "stage_completed") {
    currentIdx = 0;
  } else {
    currentIdx = globalStages.findIndex((s) => s.name === currentStage);
  }

  for (let i = 0; i < globalStages.length; i++) {
    const st = globalStages[i];
    let status;
    if (i < currentIdx) status = "completed";
    else if (i === currentIdx) status = "in_progress";
    else status = "pending";
    todos.push({
      content: globalContent(st),
      status,
      priority: "high",
    });
  }

  return todos;
}

// ── 构建 multi-plan todos ────────────────────────────────────
function buildMultiPlanTodos(statePath, stateDir, config) {
  let planFlowPath = null;
  let planFlow = null;
  try {
    const files = fs.readdirSync(stateDir);
    const flowFiles = files.filter((f) => f.endsWith("-plan-flow.json"));
    if (flowFiles.length === 1) {
      planFlowPath = path.join(stateDir, flowFiles[0]);
      planFlow = tryReadJson(planFlowPath);
    } else if (flowFiles.length > 1) {
      let baseName = path.basename(statePath);
      if (baseName.endsWith("-workflow-state.json")) {
        const moduleName = baseName.slice(0, -"-workflow-state.json".length);
        const candidate = path.join(stateDir, `${moduleName}-plan-flow.json`);
        if (fs.existsSync(candidate)) {
          planFlowPath = candidate;
          planFlow = tryReadJson(candidate);
        }
      }
    }
  } catch {
    // 目录读取失败
  }

  if (!planFlow || !planFlow.plans || !Array.isArray(planFlow.plans) || planFlow.plans.length === 0) {
    return null;
  }

  const mainState = tryReadJson(statePath) || {};
  const planStatus = mainState.plan_status || {};

  const waves = computeWaves(planFlow.plans);
  const planMeta = new Map();
  for (const p of planFlow.plans) {
    planMeta.set(p.name, {
      dependsOn: p.depends_on || [],
      path: p.path || "",
    });
  }

  const todos = [];
  const waveNums = Object.keys(waves).map(Number).sort((a, b) => a - b);

  for (const waveNum of waveNums) {
    const planNames = waves[waveNum];
    for (const name of planNames) {
      const meta = planMeta.get(name) || { dependsOn: [] };
      const planStatePath = path.join(stateDir, `${name}-state.json`);
      const planStateExists = fs.existsSync(planStatePath);
      const planState = planStateExists ? tryReadJson(planStatePath) : {};
      const flowStatus = planStatus[name] || null;
      const depsSatisfied = checkDepsSatisfied(meta.dependsOn, planStatus);

      const items = buildPlanTodoItems(
        name,
        planState,
        flowStatus,
        config,
        depsSatisfied,
        planStateExists
      );
      todos.push(...items);
    }
  }

  // 所有非 blocked plan 已 merged → 追加 global-stages 项
  const nonBlockedPlans = planFlow.plans.filter((p) => planStatus[p.name] !== "blocked");
  const allNonBlockedMerged = nonBlockedPlans.length > 0 && nonBlockedPlans.every(
    (p) => planStatus[p.name] === "merged" || planStatus[p.name] === "completed"
  );

  if (allNonBlockedMerged) {
    const globalTodos = buildGlobalStageTodos(config, mainState);
    todos.push(...globalTodos);
  }

  return todos;
}

// ── 构建 single-plan todos ───────────────────────────────────
function buildSinglePlanTodos(statePath, config) {
  const mainState = tryReadJson(statePath) || {};
  const stage = mainState.stage || "coding";
  const localStages = config["local-stages"] || [];

  function stageContent(noun, extraLabel) {
    const display = NOUN_TO_DISPLAY[noun] || noun;
    const dispatch = getStageDispatch(config, noun);
    const label = extraLabel || "";
    return `${display} — ${formatAction(dispatch)}${label}`;
  }

  // 根据 stage 推导已完成的 stage nouns
  function getCompletedNouns() {
    if (stage === "completed" || stage === "stage_completed") {
      return localStages.map((s) => s.name);
    }
    const fixing = mainState.fixing;
    const triggerStage = fixing ? fixing.trigger_stage : null;
    if (stage === "coding") return [];
    if (stage === "reviewing") return ["code"];
    if (stage === "evaluating") return ["code", "review"];
    if (stage === "fixing" || stage === "blocked") {
      if (triggerStage === "reviewing") return ["code", "review"];
      if (triggerStage === "evaluating") return ["code", "review", "evaluate"];
      return [];
    }
    return [];
  }

  const todos = [];

  // 已完成项 [completed]（content 冻结）
  for (const noun of getCompletedNouns()) {
    todos.push({
      content: stageContent(noun),
      status: "completed",
      priority: "high",
    });
  }

  // global-stage name → 已完成 local + global-stages
  const globalStages = config["global-stages"] || [];
  if (stage === "completed" || stage === "stage_completed" || globalStages.some((s) => s.name === stage)) {
    const globalTodos = buildGlobalStageTodos(config, mainState);
    todos.push(...globalTodos);
    return todos;
  }

  // blocked
  if (stage === "blocked") {
    const fixing = mainState.fixing;
    const triggerStage = fixing ? fixing.trigger_stage : null;
    let label = "🛑 blocked";
    let display = "编码";
    if (triggerStage && TRIGGER_INFO[triggerStage]) {
      const ti = TRIGGER_INFO[triggerStage];
      const round = fixing ? (fixing.round || 1) : 1;
      display = ti.display;
      label = `🛑 blocked（${ti.display}修复超限 r${round}/${ti.maxRounds}）`;
    }
    todos.push({
      content: `${display} ${label}`,
      status: "in_progress",
      priority: "high",
    });
    return todos;
  }

  // 活跃 local stage → 当前项 [in_progress]
  if (stage === "fixing") {
    const fixing = mainState.fixing;
    const triggerStage = fixing ? fixing.trigger_stage : null;
    let fixLabel = "";
    if (triggerStage && TRIGGER_INFO[triggerStage]) {
      const ti = TRIGGER_INFO[triggerStage];
      const round = fixing ? (fixing.round || 1) : 1;
      fixLabel = `（${ti.display}修复 r${round}/${ti.maxRounds}）`;
    }
    todos.push({
      content: stageContent("fix", fixLabel),
      status: "in_progress",
      priority: "high",
    });
  } else {
    const noun = stateToNoun(stage);
    if (noun) {
      todos.push({
        content: stageContent(noun),
        status: "in_progress",
        priority: "high",
      });
    }
  }

  return todos;
}

// ── 主入口 ───────────────────────────────────────────────────
function main() {
  if (process.argv.length < 3) {
    process.stderr.write("[workflow-todo-write] Usage: node workflow-todo-write.js <state-json-path>\n");
    return 1;
  }

  const statePath = process.argv[2];
  if (!fs.existsSync(statePath)) {
    process.stderr.write(`[workflow-todo-write] File not found: ${statePath}\n`);
    return 1;
  }

  let state;
  try {
    state = readJsonSync(statePath);
  } catch (exc) {
    process.stderr.write(`[workflow-todo-write] Read/parse error: ${exc}\n`);
    return 1;
  }

  if (typeof state !== "object" || state === null) {
    process.stderr.write("[workflow-todo-write] Root must be an object\n");
    return 1;
  }

  const stateDir = path.dirname(path.resolve(statePath));
  const harnessDir = path.dirname(stateDir);
  const logDir = path.join(harnessDir, "logs");

  appendLog(state, statePath, logDir);

  if (!isMainState(statePath)) {
    return 0;
  }

  const config = loadWorkflowConfig(harnessDir);
  if (!config) {
    process.stderr.write("[workflow-todo-write] 无法加载 workflow.yaml，todo 计算中止\n");
    return 1;
  }

  let todos = buildMultiPlanTodos(statePath, stateDir, config);
  if (!todos) {
    todos = buildSinglePlanTodos(statePath, config);
  }

  if (todos && todos.length > 0) {
    appendTodoCallLog(statePath, todos, logDir);
    process.stdout.write(JSON.stringify(todos));
  }

  return 0;
}

process.exit(main());
