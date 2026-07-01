#!/usr/bin/env node
/**
 * workflow-todo-write.js — Harness Workflow TodoWrite 计算 + 运行日志追加
 *
 * 在 state.json 写入后调用，完成两项工作：
 * 1. 追加结构化运行日志到 {module}-run-log.md（文件副作用，不回传 Agent）
 * 2. 计算 TodoWrite 结构化 todos[] 数组，输出到 stdout（仅对主 state 触发，回传 Agent）
 *
 * 脚本通过 stdout 回传 Agent：hook 捕获 stdout 注入到 tool output。
 * 日志写入是文件副作用，不打印到 stdout（避免噪声），仅写文件。
 *
 * TodoWrite 粒度：stage 级。每个 plan 展开为 4 项（编码/检视/评估/merge），
 * 末尾追加 1 项 "Workflow 完成"。AI 读到 [TODO] 块后直接调 todowrite({todos})。
 *
 * Exit codes: 0=成功，1=错误
 * Usage: node workflow-todo-write.js <state-json-path>
 *
 * stdout 输出格式（仅主 state，per-plan state 无 stdout 输出）：
 *     [TODO] 请立即调用 TodoWrite 工具刷新进度为以下 todos 数组：
 *
 *     ```json
 *     [
 *       {"content": "Wave 1 | Plan A: 编码", "status": "completed", "priority": "high"},
 *       {"content": "Wave 1 | Plan A: 检视 (修复 r1/3)", "status": "in_progress", "priority": "high"},
 *       ...
 *     ]
 *     ```
 *
 *     （日志写入文件，不打印到 stdout；per-plan state 写入时无 stdout 输出）
 */
const fs = require("fs");
const path = require("path");

/**
 * 读取 JSON 文件，自动去除 BOM（兼容 PowerShell utf-8 BOM 和其他编辑器写入的 BOM）。
 */
function readJsonSync(filePath) {
  let content = fs.readFileSync(filePath, "utf-8");
  if (content.charCodeAt(0) === 0xfeff || content.charCodeAt(0) === 0xfffe) {
    content = content.slice(1);
  }
  return JSON.parse(content);
}

/**
 * 安全读取 JSON 文件，失败返回 null（文件不存在或解析错误均不抛异常）。
 */
function tryReadJson(filePath) {
  try {
    return readJsonSync(filePath);
  } catch {
    return null;
  }
}

/**
 * 判断 state 文件是否为"主 state"（{module}-workflow-state.json）。
 * per-plan state 文件名为 {module}-{plan}-state.json，不含 "-workflow-" 段。
 * 主 state 是多 Plan 编排进度文件，含 plan_status 字段。
 */
function isMainState(statePath) {
  const base = path.basename(statePath);
  return base.endsWith("-workflow-state.json");
}

/**
 * 从 state 路径推断 module 名。
 * 主 state: {module}-workflow-state.json → module
 * per-plan: {module}-{plan}-state.json → module（取第一个 "-" 前）
 */
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

/**
 * 追加结构化运行日志。返回日志文件路径或 null。
 * 对所有 *-state.json 触发（主 + per-plan）。
 */
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

/**
 * 拓扑排序算 Wave 分组。
 *
 * 输入: plans 数组，每项含 name + depends_on
 * 输出: { waveNum: [planName, ...] }，waveNum 从 1 开始
 *
 * 算法: Kahn 算法变体——同一 Wave 内的 plan 依赖必须全部在前序 Wave 中。
 */
function computeWaves(plans) {
  const planMap = new Map();
  for (const p of plans) {
    planMap.set(p.name, p.depends_on || []);
  }

  const waves = {};
  const planWave = {}; // planName -> waveNum
  let currentWave = 1;

  while (planMap.size > 0) {
    // 找当前 Wave 的 plan：依赖全部已在已分配的 Wave 中
    const ready = [];
    for (const [name, deps] of planMap) {
      const allDepsResolved = deps.every((d) => planWave.hasOwnProperty(d));
      if (allDepsResolved) {
        ready.push(name);
      }
    }

    if (ready.length === 0) {
      // 循环依赖，把剩余 plan 全塞进当前 Wave 避免死循环
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

/**
 * 根据 plan state 的 stage + fixing 字段，推导该 plan 5 个编排动作 item 的 status。
 *
 * 返回 { coding, reviewing, fixing, evaluating, merge } 五项的 status 字符串
 * （completed / in_progress / pending）+ 修复项的标注（fixLabel）。
 *
 * 5 项编排动作（每项对应主 Agent 的一个编排动作）：
 *   coding     = 派发 executor(mode=coding)
 *   reviewing  = 拉起 code-review-agent
 *   fixing     = 派发 executor(mode=fix)  ← 修复是独立编排动作，非线性阶段
 *   evaluating = 拉起 code-evaluator-agent
 *   merge      = merge + 清理 worktree
 *
 * stage 流转:
 *   coding → reviewing → reviewed → evaluating → evaluated → stage_completed → completed
 *                ↓            ↓
 *             fixing       fixing
 *
 * 修复项状态语义（非线性，反复触发）:
 *   fixing 时 = in_progress（content 标注 trigger + 轮次）
 *   非 fixing = pending（未触发或本轮已结束）
 *   终态(merged) = completed（不会再修复）
 */
function deriveStageStatuses(planState, planFlowStatus, started) {
  // plan_status (主 state) 优先判定终态
  if (planFlowStatus === "merged" || planState.stage === "completed") {
    return {
      coding: "completed",
      reviewing: "completed",
      fixing: "completed",
      evaluating: "completed",
      merge: "completed",
      fixLabel: null,
    };
  }
  if (planFlowStatus === "blocked" || planState.stage === "blocked") {
    // blocked 时，用 fixing.trigger_stage 判断阻断前的进度位置
    const fixing = planState.fixing;
    const triggerStage = fixing ? fixing.trigger_stage : null;
    const result = {
      coding: "completed",
      reviewing: "pending",
      fixing: "pending",
      evaluating: "pending",
      merge: "pending",
      fixLabel: null,
    };
    if (triggerStage === "reviewing") {
      // 检视修复超限
      result.reviewing = "completed";
      result.fixing = "in_progress";
      const maxRound = 3;
      const round = fixing.round || 1;
      result.fixLabel = `🛑 检视修复超限 r${round}/${maxRound}`;
    } else if (triggerStage === "evaluating") {
      // 评估修复超限
      result.reviewing = "completed";
      result.evaluating = "completed";
      result.fixing = "in_progress";
      const maxRound = 5;
      const round = fixing.round || 1;
      result.fixLabel = `🛑 评估修复超限 r${round}/${maxRound}`;
    } else {
      // 无 fixing 信息（如编码阶段超限）
      result.coding = "in_progress";
      result.fixLabel = "🛑 blocked";
    }
    return result;
  }

  // plan 未启动（state 文件不存在）→ 全 pending
  if (!started) {
    return {
      coding: "pending",
      reviewing: "pending",
      fixing: "pending",
      evaluating: "pending",
      merge: "pending",
      fixLabel: null,
    };
  }

  const stage = planState.stage || "coding";
  const fixing = planState.fixing;

  // 基础状态（按 stage 流转线性推导）
  let coding = "pending";
  let reviewing = "pending";
  let fixingStatus = "pending";
  let evaluating = "pending";
  let merge = "pending";
  let fixLabel = null;

  switch (stage) {
    case "coding":
      coding = "in_progress";
      break;
    case "reviewing":
      coding = "completed";
      reviewing = "in_progress";
      break;
    case "fixing":
      // 修复是独立编排动作：fixing 时修复项 in_progress
      coding = "completed";
      fixingStatus = "in_progress";
      if (fixing && fixing.trigger_stage === "reviewing") {
        reviewing = "completed"; // 检视动作已执行（结果 fail）
        const maxRound = 3;
        const round = fixing.round || 1;
        fixLabel = `检视修复 r${round}/${maxRound}`;
      } else if (fixing && fixing.trigger_stage === "evaluating") {
        reviewing = "completed";
        evaluating = "completed"; // 评估动作已执行（结果 fail）
        const maxRound = 5;
        const round = fixing.round || 1;
        fixLabel = `评估修复 r${round}/${maxRound}`;
      }
      break;
    case "reviewed":
      // 检视通过（含修复后），修复项回到 pending（本轮修复已结束）
      coding = "completed";
      reviewing = "completed";
      break;
    case "evaluating":
      coding = "completed";
      reviewing = "completed";
      evaluating = "in_progress";
      break;
    case "evaluated":
      coding = "completed";
      reviewing = "completed";
      evaluating = "completed";
      break;
    case "stage_completed":
      coding = "completed";
      reviewing = "completed";
      evaluating = "completed";
      merge = "in_progress";
      break;
    default:
      coding = "in_progress";
  }

  return { coding, reviewing, fixing: fixingStatus, evaluating, merge, fixLabel };
}

/**
 * 构建单个 plan 的 5 项编排动作 todo item。
 *
 * content 明确告诉主 Agent "调什么 agent 做什么"：
 *   编码: 派发 code-executor-agent(mode=coding) 编码 {plan}
 *   检视: 拉起 code-review-agent 检视 {plan} 变更
 *   修复: 派发 code-executor-agent(mode=fix) 修复 {plan}（{trigger}修复 r{round}/{max}）
 *   评估: 拉起 code-evaluator-agent 评估 {plan} ↔ 代码
 *   merge: merge {plan} 到 main + 清理 worktree
 */
function buildPlanTodoItems(planName, waveNum, planState, planFlowStatus, dependsOn, depInfoMap, started) {
  const s = deriveStageStatuses(planState, planFlowStatus, started);

  // 依赖状态摘要
  let depStr = "";
  if (dependsOn && dependsOn.length > 0) {
    const depMarkers = dependsOn.map((dep) => {
      const depInfo = depInfoMap.get(dep);
      const depDone = depInfo ? depInfo.allCompleted : false;
      return `${dep} ${depDone ? "✅" : "🔄"}`;
    });
    depStr = ` (依赖: ${depMarkers.join(", ")})`;
  }

  const prefix = `Wave ${waveNum} | ${planName}: `;
  // 修复项 content：fixing 时带标注，否则占位
  const fixContent = s.fixLabel
    ? `${prefix}修复 — 派发 code-executor-agent(mode=fix) 修复 ${planName}（${s.fixLabel}）`
    : `${prefix}修复 — 派发 code-executor-agent(mode=fix) 修复 ${planName}`;

  return [
    {
      content: `${prefix}编码 — 派发 code-executor-agent(mode=coding) 编码 ${planName}${depStr}`,
      status: s.coding,
      priority: "high",
    },
    {
      content: `${prefix}检视 — 拉起 code-review-agent 检视 ${planName} 变更`,
      status: s.reviewing,
      priority: "high",
    },
    {
      content: fixContent,
      status: s.fixing,
      priority: "high",
    },
    {
      content: `${prefix}评估 — 拉起 code-evaluator-agent 评估 ${planName} ↔ 代码`,
      status: s.evaluating,
      priority: "high",
    },
    {
      content: `${prefix}merge — merge ${planName} 到 main + 清理 worktree`,
      status: s.merge,
      priority: "high",
    },
  ];
}

/**
 * 构建 per-plan 依赖完成度查找表。
 * 返回 Map<planName, {allCompleted: boolean}>
 */
function buildDepInfoMap(planFlow, stateDir, planStatus) {
  const depInfoMap = new Map();
  for (const p of planFlow.plans) {
    const status = planStatus[p.name] || null;
    const allCompleted = status === "merged" || status === "completed";
    depInfoMap.set(p.name, { allCompleted });
  }
  return depInfoMap;
}

/**
 * 从主 state + plan-flow.json 构建 TodoWrite todos[] 数组（多 Plan 场景）。
 *
 * 每个 plan 展开 5 项编排动作 item（编码/检视/修复/评估/merge），按 Wave 分组排序，
 * 末尾追加 "Workflow 完成: Self-Check + 总结" 项。
 */
function buildMultiPlanTodos(statePath, stateDir) {
  // 扫描同目录所有 *-plan-flow.json
  let planFlowPath = null;
  let planFlow = null;
  try {
    const files = fs.readdirSync(stateDir);
    const flowFiles = files.filter((f) => f.endsWith("-plan-flow.json"));
    if (flowFiles.length === 1) {
      planFlowPath = path.join(stateDir, flowFiles[0]);
      planFlow = tryReadJson(planFlowPath);
    } else if (flowFiles.length > 1) {
      // 多个 plan-flow：从主 state 文件名匹配 module
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
    return null; // 无 plan-flow，降级到单 Plan
  }

  // 读主 state 获取 plan_status
  const mainState = tryReadJson(statePath) || {};
  const planStatus = mainState.plan_status || {};

  // 算 Wave 分组
  const waves = computeWaves(planFlow.plans);

  // 构建依赖完成度查找表
  const depInfoMap = buildDepInfoMap(planFlow, stateDir, planStatus);

  // 构建 plan -> waveNum 映射
  const planWave = {};
  for (const waveNum of Object.keys(waves)) {
    for (const name of waves[waveNum]) {
      planWave[name] = Number(waveNum);
    }
  }

  // 构建 plan 元信息查找表
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
      // 读 per-plan state 获取精确 stage
      const planStatePath = path.join(stateDir, `${name}-state.json`);
      const planStateExists = fs.existsSync(planStatePath);
      const planState = planStateExists ? tryReadJson(planStatePath) : {};
      const flowStatus = planStatus[name] || null;

      const items = buildPlanTodoItems(
        name,
        waveNum,
        planState,
        flowStatus,
        meta.dependsOn,
        depInfoMap,
        planStateExists
      );
      todos.push(...items);
    }
  }

  // 判断是否所有 plan 已完成，决定末尾项状态
  const allPlansDone = planFlow.plans.every(
    (p) => planStatus[p.name] === "merged" || planStatus[p.name] === "completed"
  );
  const anyBlocked = planFlow.plans.some(
    (p) => planStatus[p.name] === "blocked"
  );

  todos.push({
    content: "Workflow 完成: Self-Check + 总结",
    status: allPlansDone || anyBlocked ? "in_progress" : "pending",
    priority: "medium",
  });

  return todos;
}

/**
 * 构建 TodoWrite todos[] 数组（单 Plan 场景）。
 *
 * 主 state 本身就是该 plan 的 state（stage 字段直接用）。
 * 5 项编排动作 item + 1 项 Workflow 完成。
 */
function buildSinglePlanTodos(statePath) {
  const mainState = tryReadJson(statePath) || {};
  const s = deriveStageStatuses(mainState, null, true);

  const fixContent = s.fixLabel
    ? `修复 — 派发 code-executor-agent(mode=fix) 修复（${s.fixLabel}）`
    : "修复 — 派发 code-executor-agent(mode=fix) 修复";

  const todos = [
    {
      content: "编码 — 派发 code-executor-agent(mode=coding) 编码",
      status: s.coding,
      priority: "high",
    },
    {
      content: "检视 — 拉起 code-review-agent 检视变更",
      status: s.reviewing,
      priority: "high",
    },
    {
      content: fixContent,
      status: s.fixing,
      priority: "high",
    },
    {
      content: "评估 — 拉起 code-evaluator-agent 评估 Plan ↔ 代码",
      status: s.evaluating,
      priority: "high",
    },
    {
      content: "Workflow 完成: Self-Check + 总结",
      status: mainState.stage === "stage_completed" || mainState.stage === "completed"
        ? "in_progress"
        : "pending",
      priority: "medium",
    },
  ];

  return todos;
}

/**
 * 主入口：读 state.json → 追加日志 →（仅主 state）计算 todos[] 输出。
 */
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

  // 推断 harness 目录结构
  const stateDir = path.dirname(path.resolve(statePath));
  const harnessDir = path.dirname(stateDir);
  const logDir = path.join(harnessDir, "logs");

  // 1. 追加运行日志（文件副作用，不回传 Agent——日志已落盘，打印 stdout 纯属噪声）
  appendLog(state, statePath, logDir);

  // 2. 计算 todos[]（仅对主 state 触发——per-plan state 不刷主 Agent todo）
  if (!isMainState(statePath)) {
    return 0;
  }

  let todos = buildMultiPlanTodos(statePath, stateDir);
  if (!todos) {
    // 无 plan-flow 或 plans 为空 → 单 Plan 场景
    todos = buildSinglePlanTodos(statePath);
  }

  if (todos && todos.length > 0) {
    console.log("[TODO] 请立即调用 TodoWrite 工具刷新进度为以下 todos 数组：");
    console.log("");
    console.log("```json");
    console.log(JSON.stringify(todos, null, 2));
    console.log("```");
  }

  return 0;
}

process.exit(main());
