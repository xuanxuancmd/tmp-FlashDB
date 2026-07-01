#!/usr/bin/env node
/**
 * eval-workflow-todo.js — workflow-todo-write.js 正确性验证
 *
 * 核心原则：
 * - 已完成项 content 冻结不改，只改 status=completed
 * - 初始每 plan 1 项 code [pending]，随推进增量增长（非全景投影）
 * - merge 不是 stage，不产生 todo 项
 * - 支持自定义 stage（如 codecheck），验证动态推导正确性
 * - max_rounds 从 config.toml 统一读取，所有 stage 共用同一个值
 *
 * fixture 策略：workflow.yaml / config.toml / plan-flow.json 内置在 eval/fixtures/ 下，
 * 不在脚本中动态生成。测试场景的 state.json 在脚本中构造（按场景定制）。
 *
 * Usage: node eval-workflow-todo.js [--quiet]
 * Exit: 0=全部通过, 1=有失败
 */
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const EVAL_DIR = __dirname;
const SCRIPT_PATH = path.join(EVAL_DIR, "..", "references", "scripts", "workflow-todo-write.js");
const FIXTURES_DIR = path.join(EVAL_DIR, "fixtures");
const TMP_DIR = path.join(EVAL_DIR, ".tmp");
const MODULE = "testmod";

// 从 fixtures 读取固定配置
const WORKFLOW_YAML = fs.readFileSync(path.join(FIXTURES_DIR, "workflow.yaml"), "utf-8");
const CONFIG_TOML = fs.readFileSync(path.join(FIXTURES_DIR, "config.toml"), "utf-8");
const PLAN_FLOW = JSON.parse(fs.readFileSync(path.join(FIXTURES_DIR, "plan-flow.json"), "utf-8"));

// content 片段常量（与 fixture workflow.yaml 中的 stage/skill 对应）
const CODE = "编码 — 派发 code-executor-agent sub-Agent";
const CODECHECK = "codecheck — 加载并执行 Skill custom-codecheck-skill";
const REVIEW = "检视 — 加载并执行 Skill harness-code-review";
const EVAL = "评估 — 派发 code-evaluator-agent sub-Agent";  // evaluate 改为 agent 直接派发
const FIX = "修复 — 派发 code-executor-agent sub-Agent";
const GLOBAL = "检视 — 加载并执行 Skill harness-run-e2e-test";

function parseTodoJson(stdout) {
  const trimmed = stdout.trim();
  if (!trimmed.startsWith("[")) throw new Error(`stdout 不是 JSON:\n${stdout}`);
  return JSON.parse(trimmed);
}

function setupScenarioDir(scenario, idx) {
  const scenarioDir = path.join(TMP_DIR, String(idx));
  const harnessDir = path.join(scenarioDir, "harness");
  const stateDir = path.join(harnessDir, "state");
  fs.mkdirSync(stateDir, { recursive: true });
  fs.writeFileSync(path.join(harnessDir, "workflow.yaml"), WORKFLOW_YAML, "utf-8");
  fs.writeFileSync(path.join(harnessDir, "config.toml"), CONFIG_TOML, "utf-8");
  if (!scenario.noPlanFlow) {
    fs.writeFileSync(path.join(stateDir, `${MODULE}-plan-flow.json`), JSON.stringify(PLAN_FLOW), "utf-8");
  }
  const mainStatePath = path.join(stateDir, `${MODULE}-workflow-state.json`);
  fs.writeFileSync(mainStatePath, JSON.stringify(scenario.mainState), "utf-8");
  for (const [name, st] of Object.entries(scenario.planStates || {})) {
    fs.writeFileSync(path.join(stateDir, `${name}-state.json`), JSON.stringify(st), "utf-8");
  }
  return mainStatePath;
}

function runScenario(scenario, idx) {
  const mainStatePath = setupScenarioDir(scenario, idx);
  let stdout, stderr;
  try {
    stdout = execFileSync("node", [SCRIPT_PATH, mainStatePath], { encoding: "utf-8" });
  } catch (exc) {
    stdout = exc.stdout || "";
    stderr = exc.stderr || "";
  }
  return { todos: stdout.trim().startsWith("[") ? parseTodoJson(stdout) : [], stdout, stderr };
}

// 断言：逐项检查 content 子串 + status
function assertTodos(todos, expected, scenarioName) {
  const errors = [];
  if (todos.length !== expected.items.length) {
    errors.push(`  todo 数量: 期望 ${expected.items.length}, 实际 ${todos.length}`);
  }
  for (let i = 0; i < expected.items.length; i++) {
    const exp = expected.items[i];
    const todo = todos[i];
    if (!todo) { errors.push(`  [item ${i}] 缺失`); continue; }
    const checks = Array.isArray(exp.c) ? exp.c : [exp.c];
    for (const c of checks) {
      if (!todo.content.includes(c)) errors.push(`  [item ${i}] 不含 "${c}" → "${todo.content}"`);
    }
    if (exp.s && todo.status !== exp.s) errors.push(`  [item ${i}] status: 期望 ${exp.s}, 实际 ${todo.status}`);
    if (exp.nc && todo.content.includes(exp.nc)) errors.push(`  [item ${i}] 不应含 "${exp.nc}" → "${todo.content}"`);
  }
  return errors;
}

// ============================================================
// 测试场景
// max_rounds = 3（从 config.toml 统一读取，所有 stage 共用）
// ============================================================

const scenarios = [
  // ── 多 Plan 场景 ──────────────────────────────────
  {
    name: "场景1: 全新启动 — 所有 plan 各 1 项 code [pending]",
    mainState: { truth_source: "p1.md", stage: "coding",
      plan_status: { plan1: "running", plan2: "running", plan3: "running", plan4: "running" } },
    planStates: { plan1: { truth_source: "p1.md", stage: "coding" } },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "in_progress" },
      { c: ["plan2: ", CODE], s: "pending" },
      { c: ["plan3: ", CODE], s: "pending" },
      { c: ["plan4: ", CODE], s: "pending" },
    ]},
  },
  {
    name: "场景2: plan1 merged, plan2/3 编码中 — plan1 全部 [completed]（含 codecheck）",
    mainState: { truth_source: "p1.md", stage: "coding",
      plan_status: { plan1: "merged", plan2: "running", plan3: "running", plan4: "running" } },
    planStates: {
      plan2: { truth_source: "p2.md", stage: "coding" },
      plan3: { truth_source: "p3.md", stage: "coding" },
    },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", CODECHECK], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "in_progress" },
      { c: ["plan3: ", CODE], s: "in_progress" },
      { c: ["plan4: ", CODE], s: "pending" },
    ]},
  },
  {
    name: "场景3: plan2 codecheck 修复中 (r1/3), plan3 review 中 — 验证 codecheck 修复标注",
    mainState: { truth_source: "p1.md", stage: "codecheck",
      plan_status: { plan1: "merged", plan2: "running", plan3: "running", plan4: "running" } },
    planStates: {
      plan2: { truth_source: "p2.md", stage: "fixing", fixing: { trigger_stage: "codecheck", round: 1 } },
      plan3: { truth_source: "p3.md", stage: "reviewing" },
    },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", CODECHECK], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", CODECHECK], s: "completed" },
      { c: ["plan2: ", FIX, "codecheck修复 r1/3"], s: "in_progress" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", CODECHECK], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "in_progress" },
      { c: ["plan4: ", CODE], s: "pending" },
    ]},
  },
  {
    name: "场景4: 所有 plan stage_completed — 全部 local [completed]（含 codecheck）",
    mainState: { truth_source: "p1.md", stage: "stage_completed",
      plan_status: { plan1: "running", plan2: "running", plan3: "running", plan4: "running" } },
    planStates: {
      plan1: { truth_source: "p1.md", stage: "stage_completed" },
      plan2: { truth_source: "p2.md", stage: "stage_completed" },
      plan3: { truth_source: "p3.md", stage: "stage_completed" },
      plan4: { truth_source: "p4.md", stage: "stage_completed" },
    },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", CODECHECK], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", CODECHECK], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", CODECHECK], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "completed" },
      { c: ["plan4: ", CODE], s: "completed" },
      { c: ["plan4: ", CODECHECK], s: "completed" },
      { c: ["plan4: ", REVIEW], s: "completed" },
      { c: ["plan4: ", EVAL], s: "completed" },
    ]},
  },
  {
    name: "场景5: 全部 merged — local 全 [completed] + global [in_progress]",
    mainState: { truth_source: "p1.md", stage: "stage_completed",
      plan_status: { plan1: "merged", plan2: "merged", plan3: "merged", plan4: "merged" } },
    planStates: {},
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", CODECHECK], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", CODECHECK], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", CODECHECK], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "completed" },
      { c: ["plan4: ", CODE], s: "completed" },
      { c: ["plan4: ", CODECHECK], s: "completed" },
      { c: ["plan4: ", REVIEW], s: "completed" },
      { c: ["plan4: ", EVAL], s: "completed" },
      { c: GLOBAL, s: "in_progress" },
    ]},
  },
  {
    name: "场景6: completed — 全 [completed] + Workflow 完成",
    mainState: { truth_source: "p1.md", stage: "completed",
      plan_status: { plan1: "merged", plan2: "merged", plan3: "merged", plan4: "merged" } },
    planStates: {},
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", CODECHECK], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", CODECHECK], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", CODECHECK], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "completed" },
      { c: ["plan4: ", CODE], s: "completed" },
      { c: ["plan4: ", CODECHECK], s: "completed" },
      { c: ["plan4: ", REVIEW], s: "completed" },
      { c: ["plan4: ", EVAL], s: "completed" },
      { c: GLOBAL, s: "completed" },
      { c: "Workflow 完成", s: "in_progress" },
    ]},
  },
  {
    name: "场景7: plan3 blocked (trigger=codecheck) — 验证 codecheck 修复超限标注",
    mainState: { truth_source: "p1.md", stage: "codecheck",
      plan_status: { plan1: "merged", plan2: "merged", plan3: "blocked", plan4: "running" } },
    planStates: {
      plan3: { truth_source: "p3.md", stage: "blocked",
        fixing: { trigger_stage: "codecheck", round: 3 }, blocked_reason: "exceeded" },
    },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", CODECHECK], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", CODECHECK], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", CODECHECK], s: "completed" },
      { c: ["plan3:", "blocked", "codecheck修复超限 r3/3"], s: "in_progress" },
      { c: ["plan4: ", CODE], s: "pending" },
    ]},
  },

  // ── 单 Plan 场景 ──────────────────────────────────
  {
    name: "场景8: 单 Plan 编码中 — 1 项 [in_progress]",
    mainState: { truth_source: "p1.md", stage: "coding" },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "in_progress", nc: "plan" },
    ]},
  },
  {
    name: "场景9: 单 Plan codecheck 中 — code [completed] + codecheck [in_progress]",
    mainState: { truth_source: "p1.md", stage: "codecheck" },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "completed", nc: "plan" },
      { c: CODECHECK, s: "in_progress", nc: "plan" },
    ]},
  },
  {
    name: "场景10: 单 Plan codecheck 修复中 (trigger=codecheck, r1/3)",
    mainState: { truth_source: "p1.md", stage: "fixing",
      fixing: { trigger_stage: "codecheck", round: 1 } },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "completed", nc: "plan" },
      { c: CODECHECK, s: "completed", nc: "plan" },
      { c: [FIX, "codecheck修复 r1/3"], s: "in_progress", nc: "plan" },
    ]},
  },
  {
    name: "场景11: 单 Plan review 修复中 (trigger=reviewing, r2/3) — codecheck 已通过",
    mainState: { truth_source: "p1.md", stage: "fixing",
      fixing: { trigger_stage: "reviewing", round: 2 } },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "completed", nc: "plan" },
      { c: CODECHECK, s: "completed", nc: "plan" },
      { c: REVIEW, s: "completed", nc: "plan" },
      { c: [FIX, "检视修复 r2/3"], s: "in_progress", nc: "plan" },
    ]},
  },
  {
    name: "场景12: 单 Plan evaluate 修复中 (trigger=evaluating, r3/3)",
    mainState: { truth_source: "p1.md", stage: "fixing",
      fixing: { trigger_stage: "evaluating", round: 3 } },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "completed", nc: "plan" },
      { c: CODECHECK, s: "completed", nc: "plan" },
      { c: REVIEW, s: "completed", nc: "plan" },
      { c: EVAL, s: "completed", nc: "plan" },
      { c: [FIX, "评估修复 r3/3"], s: "in_progress", nc: "plan" },
    ]},
  },
  {
    name: "场景13: 单 Plan evaluate 中 — code+codecheck+review [completed] + evaluate [in_progress]",
    mainState: { truth_source: "p1.md", stage: "evaluating" },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "completed", nc: "plan" },
      { c: CODECHECK, s: "completed", nc: "plan" },
      { c: REVIEW, s: "completed", nc: "plan" },
      { c: EVAL, s: "in_progress", nc: "plan" },
    ]},
  },
];

// ============================================================
// 运行
// ============================================================

const QUIET = process.argv.includes("--quiet");

function main() {
  let pass = 0, fail = 0;
  if (fs.existsSync(TMP_DIR)) fs.rmSync(TMP_DIR, { recursive: true, force: true });
  fs.mkdirSync(TMP_DIR, { recursive: true });

  try {
    for (let idx = 0; idx < scenarios.length; idx++) {
      const scenario = scenarios[idx];
      try {
        const result = runScenario(scenario, idx);
        const errors = assertTodos(result.todos, scenario.expected, scenario.name);
        if (!QUIET) {
          console.log(`=== ${scenario.name} ===`);
          result.todos.forEach((todo, i) => {
            const icon = { completed: "[✓]", in_progress: "[→]", pending: "[ ]" }[todo.status] || "[?]";
            console.log(`  ${String(i).padStart(2, "0")} ${icon} ${todo.content}`);
          });
          if (result.stderr) console.log(`  stderr: ${result.stderr.trim()}`);
          console.log("");
        }
        if (errors.length === 0) { console.log(`✓ ${scenario.name}`); pass++; }
        else { console.log(`✗ ${scenario.name}`); errors.forEach((e) => console.log(e)); fail++; }
      } catch (exc) {
        console.log(`✗ ${scenario.name}\n  异常: ${exc.message || exc}`);
        fail++;
      }
      if (!QUIET) console.log("");
    }
  } finally {
    if (fs.existsSync(TMP_DIR)) fs.rmSync(TMP_DIR, { recursive: true, force: true });
  }
  console.log(`${"=".repeat(70)}\n结果: ${pass}/${pass + fail} 通过`);
  return fail === 0 ? 0 : 1;
}

process.exit(main());
