#!/usr/bin/env node
/**
 * eval-workflow-todo.js — workflow-todo-write.js 正确性验证
 *
 * 核心原则：
 * - 已完成项 content 冻结不改，只改 status=completed
 * - 初始每 plan 1 项 code [pending]，随推进增量增长（非全景投影）
 * - merge 不是 stage，不产生 todo 项
 *
 * 临时文件策略：写入 eval/.tmp/，运行后清理
 *
 * Usage: node eval-workflow-todo.js [--quiet]
 * Exit: 0=全部通过, 1=有失败
 */
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const EVAL_DIR = __dirname;
const SCRIPT_PATH = path.join(EVAL_DIR, "..", "references", "scripts", "workflow-todo-write.js");
const TMP_DIR = path.join(EVAL_DIR, ".tmp");
const MODULE = "testmod";

const WORKFLOW_YAML = `version: 1
module: ${MODULE}

local-stages:
  - name: code
    agent: code-executor-agent
  - name: review
    skill: harness-code-review
    on_failure: fix
  - name: evaluate
    skill: harness-code-evaluator
    on_failure: fix

global-stages:
  - name: review
    skill: harness-run-e2e-test
    on_failure: fix

optional-stages:
  fix:
    agent: code-executor-agent
`;

const PLAN_FLOW = {
  plans: [
    { name: "plan1", path: "p1.md", depends_on: [] },
    { name: "plan2", path: "p2.md", depends_on: ["plan1"] },
    { name: "plan3", path: "p3.md", depends_on: ["plan1"] },
    { name: "plan4", path: "p4.md", depends_on: ["plan2", "plan3"] },
  ],
};

// content 片段常量
const CODE = "编码 — 派发 code-executor-agent sub-Agent";
const REVIEW = "检视 — 加载并执行 Skill harness-code-review";
const EVAL = "评估 — 加载并执行 Skill harness-code-evaluator";
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
// ============================================================

const scenarios = [
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
    name: "场景2: plan1 merged, plan2/3 编码中 — plan1 全部 [completed]",
    mainState: { truth_source: "p1.md", stage: "coding",
      plan_status: { plan1: "merged", plan2: "running", plan3: "running", plan4: "running" } },
    planStates: {
      plan2: { truth_source: "p2.md", stage: "coding" },
      plan3: { truth_source: "p3.md", stage: "coding" },
    },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "in_progress" },
      { c: ["plan3: ", CODE], s: "in_progress" },
      { c: ["plan4: ", CODE], s: "pending" },
    ]},
  },
  {
    name: "场景3: plan2 修复中, plan3 评估中 — 已完成项 content 冻结",
    mainState: { truth_source: "p1.md", stage: "reviewing",
      plan_status: { plan1: "merged", plan2: "running", plan3: "running", plan4: "running" } },
    planStates: {
      plan2: { truth_source: "p2.md", stage: "fixing", fixing: { trigger_stage: "reviewing", round: 2 } },
      plan3: { truth_source: "p3.md", stage: "evaluating" },
    },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", FIX, "检视修复 r2/3"], s: "in_progress" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "in_progress" },
      { c: ["plan4: ", CODE], s: "pending" },
    ]},
  },
  {
    name: "场景4: 所有 plan stage_completed — 全部 local [completed], 无 merge 项",
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
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "completed" },
      { c: ["plan4: ", CODE], s: "completed" },
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
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "completed" },
      { c: ["plan4: ", CODE], s: "completed" },
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
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "completed" },
      { c: ["plan4: ", CODE], s: "completed" },
      { c: ["plan4: ", REVIEW], s: "completed" },
      { c: ["plan4: ", EVAL], s: "completed" },
      { c: GLOBAL, s: "completed" },
      { c: "Workflow 完成", s: "in_progress" },
    ]},
  },
  {
    name: "场景7: plan3 blocked — 已完成项保留 + blocked 项",
    mainState: { truth_source: "p1.md", stage: "reviewing",
      plan_status: { plan1: "merged", plan2: "merged", plan3: "blocked", plan4: "running" } },
    planStates: {
      plan3: { truth_source: "p3.md", stage: "blocked",
        fixing: { trigger_stage: "evaluating", round: 5 }, blocked_reason: "exceeded" },
    },
    expected: { items: [
      { c: ["plan1: ", CODE], s: "completed" },
      { c: ["plan1: ", REVIEW], s: "completed" },
      { c: ["plan1: ", EVAL], s: "completed" },
      { c: ["plan2: ", CODE], s: "completed" },
      { c: ["plan2: ", REVIEW], s: "completed" },
      { c: ["plan2: ", EVAL], s: "completed" },
      { c: ["plan3: ", CODE], s: "completed" },
      { c: ["plan3: ", REVIEW], s: "completed" },
      { c: ["plan3: ", EVAL], s: "completed" },
      { c: ["plan3:", "blocked", "评估修复超限 r5/5"], s: "in_progress" },
      { c: ["plan4: ", CODE], s: "pending" },
    ]},
  },
  {
    name: "场景8: 单 Plan 编码中 — 1 项 [in_progress]",
    mainState: { truth_source: "p1.md", stage: "coding" },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "in_progress", nc: "plan" },
    ]},
  },
  {
    name: "场景9: 单 Plan 评估中 — code+review [completed] + evaluate [in_progress]",
    mainState: { truth_source: "p1.md", stage: "evaluating" },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "completed", nc: "plan" },
      { c: REVIEW, s: "completed", nc: "plan" },
      { c: EVAL, s: "in_progress", nc: "plan" },
    ]},
  },
  {
    name: "场景10: 单 Plan 修复中 (trigger=reviewing, r2/3)",
    mainState: { truth_source: "p1.md", stage: "fixing",
      fixing: { trigger_stage: "reviewing", round: 2 } },
    planStates: {}, noPlanFlow: true,
    expected: { items: [
      { c: CODE, s: "completed", nc: "plan" },
      { c: REVIEW, s: "completed", nc: "plan" },
      { c: [FIX, "检视修复 r2/3"], s: "in_progress", nc: "plan" },
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
