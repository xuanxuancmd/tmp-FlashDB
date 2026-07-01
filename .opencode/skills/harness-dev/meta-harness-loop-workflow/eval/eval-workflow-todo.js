#!/usr/bin/env node
/**
 * eval-workflow-todo.js — workflow-todo-write.js 正确性验证
 *
 * 依赖链: plan1 → plan2+plan3（可并行）→ plan4
 *   Wave 1: plan1
 *   Wave 2: plan2, plan3
 *   Wave 3: plan4
 *
 * 验证：给定 plan-flow.json + 某 plan 的 state 进展，
 * 断言脚本输出的 todos[] 数组（content + status）符合预期。
 *
 * 屏显：每个场景打印输入（plan-flow.json + 各 state.json）和输出（脚本 stdout + 解析后的 todos）。
 *
 * Usage: node eval-workflow-todo.js
 *        node eval-workflow-todo.js --quiet   # 仅打印结果，不打印输入输出
 * Exit: 0=全部通过, 1=有失败
 */
const fs = require("fs");
const path = require("path");
const os = require("os");
const { execFileSync } = require("child_process");

const SCRIPT_PATH = path.join(__dirname, "..", "references", "scripts", "workflow-todo-write.js");
const MODULE = "testmod";

/**
 * 公共 plan-flow.json: plan1 → plan2+plan3 → plan4
 */
const PLAN_FLOW = {
  plans: [
    { name: "plan1", path: "p1.md", depends_on: [] },
    { name: "plan2", path: "p2.md", depends_on: ["plan1"] },
    { name: "plan3", path: "p3.md", depends_on: ["plan1"] },
    { name: "plan4", path: "p4.md", depends_on: ["plan2", "plan3"] },
  ],
};

/**
 * 5 个编排动作的中文名（与脚本输出的 content 一致），用于断言 content 片段。
 * 顺序: 编码 / 检视 / 修复 / 评估 / merge
 */
const STAGE_LABELS = ["编码", "检视", "修复", "评估", "merge"];

/**
 * 从 stdout 提取 [TODO] 块中的 ```json ... ``` 数组并解析。
 */
function parseTodoJson(stdout) {
  const match = stdout.match(/```json\n([\s\S]*?)\n```/);
  if (!match) {
    throw new Error(`stdout 中未找到 JSON 块:\n${stdout}`);
  }
  return JSON.parse(match[1]);
}

/**
 * 格式化 JSON 为缩进字符串（用于屏显）。
 */
function prettyJson(obj) {
  return JSON.stringify(obj, null, 2);
}

/**
 * 运行单个场景：构造临时目录 → 写文件 → 调脚本 → 返回 { todos, stdout, inputSummary }。
 */
function runScenario(scenario) {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "wf-eval-"));
  const stateDir = path.join(tmpDir, "state");
  fs.mkdirSync(stateDir, { recursive: true });

  // 写 plan-flow.json
  fs.writeFileSync(
    path.join(stateDir, `${MODULE}-plan-flow.json`),
    JSON.stringify(PLAN_FLOW),
    "utf-8"
  );

  // 写主 state
  const mainStatePath = path.join(stateDir, `${MODULE}-workflow-state.json`);
  fs.writeFileSync(mainStatePath, JSON.stringify(scenario.mainState), "utf-8");

  // 写 per-plan state（仅写 scenario 提供的）
  for (const [name, st] of Object.entries(scenario.planStates || {})) {
    fs.writeFileSync(
      path.join(stateDir, `${name}-state.json`),
      JSON.stringify(st),
      "utf-8"
    );
  }

  // 记录输入摘要（用于屏显）
  const inputSummary = {
    planFlow: PLAN_FLOW,
    mainState: scenario.mainState,
    planStates: scenario.planStates || {},
  };

  // 调用 workflow-todo-write.js
  const stdout = execFileSync("node", [SCRIPT_PATH, mainStatePath], {
    encoding: "utf-8",
  });

  // 清理临时目录
  fs.rmSync(tmpDir, { recursive: true, force: true });

  return {
    todos: parseTodoJson(stdout),
    stdout,
    inputSummary,
  };
}

/**
 * 断言 todos 数组符合预期。
 *
 * expected 格式:
 *   plans: [{ name, wave, statuses: [编码, 检视, 评估, merge], labelCheck: {idx: "substring"} (可选) }]
 *   final: "pending" | "in_progress" | "completed"
 *   count: 17 (4 plans × 4 + 1)
 */
function assertTodos(todos, expected, scenarioName) {
  const errors = [];
  let idx = 0;

  for (const p of expected.plans) {
    for (let i = 0; i < 5; i++) {
      const todo = todos[idx];
      if (!todo) {
        errors.push(`  [${p.name}.${STAGE_LABELS[i]}] todo 缺失 (idx=${idx})`);
        idx++;
        continue;
      }
      // content 必须含 "Wave {wave} | {name}: {stageLabel}"
      const expectedContent = `Wave ${p.wave} | ${p.name}: ${STAGE_LABELS[i]}`;
      if (!todo.content.includes(expectedContent)) {
        errors.push(
          `  [${p.name}.${STAGE_LABELS[i]}] content 不含 "${expectedContent}" → "${todo.content}"`
        );
      }
      // status
      if (todo.status !== p.statuses[i]) {
        errors.push(
          `  [${p.name}.${STAGE_LABELS[i]}] status: 期望 ${p.statuses[i]}, 实际 ${todo.status}`
        );
      }
      // 可选 label 检查（如修复轮次标注）
      if (p.labelCheck && p.labelCheck[i] && !todo.content.includes(p.labelCheck[i])) {
        errors.push(
          `  [${p.name}.${STAGE_LABELS[i]}] label 不含 "${p.labelCheck[i]}" → "${todo.content}"`
        );
      }
      idx++;
    }
  }

  // 末尾项
  const finalTodo = todos[idx];
  if (!finalTodo) {
    errors.push(`  [final] todo 缺失 (idx=${idx})`);
  } else {
    if (!finalTodo.content.includes("Workflow 完成")) {
      errors.push(`  [final] content 不含 "Workflow 完成" → "${finalTodo.content}"`);
    }
    if (finalTodo.status !== expected.final) {
      errors.push(`  [final] status: 期望 ${expected.final}, 实际 ${finalTodo.status}`);
    }
    idx++;
  }

  // 总数
  if (todos.length !== idx) {
    errors.push(`  todo 总数: 期望 ${idx}, 实际 ${todos.length}`);
  }

  return errors;
}

// ============================================================
// 测试场景
// ============================================================

const scenarios = [
  {
    name: "场景1: 全新启动 — plan1 编码中, 其余未启动",
    mainState: {
      truth_source: "p1.md",
      stage: "coding",
      plan_status: { plan1: "running", plan2: "running", plan3: "running", plan4: "running" },
    },
    planStates: {
      plan1: { truth_source: "p1.md", stage: "coding" },
      // plan2/3/4 无 state 文件 → 未启动
    },
    expected: {
      plans: [
        { name: "plan1", wave: 1, statuses: ["in_progress", "pending", "pending", "pending", "pending"] },
        { name: "plan2", wave: 2, statuses: ["pending", "pending", "pending", "pending", "pending"] },
        { name: "plan3", wave: 2, statuses: ["pending", "pending", "pending", "pending", "pending"] },
        { name: "plan4", wave: 3, statuses: ["pending", "pending", "pending", "pending", "pending"] },
      ],
      final: "pending",
    },
  },
  {
    name: "场景2: plan1 检视中, plan2/plan3 并行编码中",
    mainState: {
      truth_source: "p1.md",
      stage: "reviewing",
      plan_status: { plan1: "running", plan2: "running", plan3: "running", plan4: "running" },
    },
    planStates: {
      plan1: { truth_source: "p1.md", stage: "reviewing" },
      plan2: { truth_source: "p2.md", stage: "coding" },
      plan3: { truth_source: "p3.md", stage: "coding" },
    },
    expected: {
      plans: [
        { name: "plan1", wave: 1, statuses: ["completed", "in_progress", "pending", "pending", "pending"] },
        { name: "plan2", wave: 2, statuses: ["in_progress", "pending", "pending", "pending", "pending"] },
        { name: "plan3", wave: 2, statuses: ["in_progress", "pending", "pending", "pending", "pending"] },
        { name: "plan4", wave: 3, statuses: ["pending", "pending", "pending", "pending", "pending"] },
      ],
      final: "pending",
    },
  },
  {
    name: "场景3: plan1 merged, plan2 检视修复 r2/3, plan3 评估中, plan4 待启动",
    mainState: {
      truth_source: "p1.md",
      stage: "reviewing",
      plan_status: { plan1: "merged", plan2: "running", plan3: "running", plan4: "running" },
    },
    planStates: {
      plan1: { truth_source: "p1.md", stage: "completed" },
      plan2: {
        truth_source: "p2.md",
        stage: "fixing",
        fixing: { trigger_stage: "reviewing", round: 2 },
      },
      plan3: { truth_source: "p3.md", stage: "evaluating" },
    },
    expected: {
      plans: [
        { name: "plan1", wave: 1, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        {
          name: "plan2",
          wave: 2,
          statuses: ["completed", "completed", "in_progress", "pending", "pending"],
          labelCheck: { 2: "检视修复 r2/3" },
        },
        { name: "plan3", wave: 2, statuses: ["completed", "completed", "pending", "in_progress", "pending"] },
        { name: "plan4", wave: 3, statuses: ["pending", "pending", "pending", "pending", "pending"] },
      ],
      final: "pending",
    },
  },
  {
    name: "场景4: plan1/2/3 merged, plan4 编码中",
    mainState: {
      truth_source: "p1.md",
      stage: "coding",
      plan_status: { plan1: "merged", plan2: "merged", plan3: "merged", plan4: "running" },
    },
    planStates: {
      plan4: { truth_source: "p4.md", stage: "coding" },
    },
    expected: {
      plans: [
        { name: "plan1", wave: 1, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        { name: "plan2", wave: 2, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        { name: "plan3", wave: 2, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        { name: "plan4", wave: 3, statuses: ["in_progress", "pending", "pending", "pending", "pending"] },
      ],
      final: "pending",
    },
  },
  {
    name: "场景5: 全部 merged — 末尾项应 in_progress",
    mainState: {
      truth_source: "p1.md",
      stage: "completed",
      plan_status: { plan1: "merged", plan2: "merged", plan3: "merged", plan4: "merged" },
    },
    planStates: {},
    expected: {
      plans: [
        { name: "plan1", wave: 1, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        { name: "plan2", wave: 2, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        { name: "plan3", wave: 2, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        { name: "plan4", wave: 3, statuses: ["completed", "completed", "completed", "completed", "completed"] },
      ],
      final: "in_progress",
    },
  },
  {
    name: "场景6: plan3 blocked (评估修复超限) — 末尾项应 in_progress",
    mainState: {
      truth_source: "p1.md",
      stage: "reviewing",
      plan_status: { plan1: "merged", plan2: "merged", plan3: "blocked", plan4: "running" },
    },
    planStates: {
      plan3: {
        truth_source: "p3.md",
        stage: "blocked",
        fixing: { trigger_stage: "evaluating", round: 5 },
        blocked_reason: "evaluator max rounds exceeded",
      },
    },
    expected: {
      plans: [
        { name: "plan1", wave: 1, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        { name: "plan2", wave: 2, statuses: ["completed", "completed", "completed", "completed", "completed"] },
        {
          name: "plan3",
          wave: 2,
          statuses: ["completed", "completed", "in_progress", "completed", "pending"],
          labelCheck: { 2: "评估修复超限 r5/5" },
        },
        { name: "plan4", wave: 3, statuses: ["pending", "pending", "pending", "pending", "pending"] },
      ],
      final: "in_progress",
    },
  },
];

// ============================================================
// 运行
// ============================================================

const QUIET = process.argv.includes("--quiet");

/**
 * 打印分隔线。
 */
function printSeparator(char = "=", len = 70) {
  console.log(char.repeat(len));
}

function main() {
  let pass = 0;
  let fail = 0;

  for (const scenario of scenarios) {
    try {
      const result = runScenario(scenario);
      const errors = assertTodos(result.todos, scenario.expected, scenario.name);

      if (!QUIET) {
        printSeparator("=");
        console.log(`场景: ${scenario.name}`);
        printSeparator("-");

        // 输入
        console.log("【输入】");
        console.log("plan-flow.json:");
        console.log(prettyJson(result.inputSummary.planFlow));
        console.log("");
        console.log("主 state (" + MODULE + "-workflow-state.json):");
        console.log(prettyJson(result.inputSummary.mainState));
        console.log("");
        console.log("per-plan state:");
        const planStateKeys = Object.keys(result.inputSummary.planStates);
        if (planStateKeys.length === 0) {
          console.log("  (无 per-plan state 文件 — plan 均未启动)");
        } else {
          for (const name of planStateKeys) {
            console.log(`  ${name}-state.json:`);
            console.log(prettyJson(result.inputSummary.planStates[name])
              .split("\n").map((l) => "    " + l).join("\n"));
          }
        }
        console.log("");

        // 输出
        console.log("【输出】workflow-todo-write.js stdout:");
        console.log(result.stdout.trim());
        console.log("");

        // 解析后的 todos
        console.log("【解析】todos[] 数组:");
        result.todos.forEach((todo, i) => {
          const statusIcon = {
            completed: "[✓]",
            in_progress: "[→]",
            pending: "[ ]",
          }[todo.status] || "[?]";
          console.log(`  ${String(i).padStart(2, "0")} ${statusIcon} ${todo.content}`);
        });
        console.log("");
      }

      if (errors.length === 0) {
        console.log(`\u2713 ${scenario.name}`);
        pass++;
      } else {
        console.log(`\u2717 ${scenario.name}`);
        for (const e of errors) {
          console.log(e);
        }
        fail++;
      }
    } catch (exc) {
      if (!QUIET) {
        printSeparator("=");
        console.log(`场景: ${scenario.name}`);
        printSeparator("-");
        console.log("【异常】脚本执行失败:");
        if (exc.stderr) {
          console.log("stderr:", exc.stderr);
        }
        console.log("error:", exc.message || exc);
        console.log("");
      }
      console.log(`\u2717 ${scenario.name}`);
      console.log(`  异常: ${exc.message || exc}`);
      fail++;
    }

    if (!QUIET) {
      console.log("");
    }
  }

  printSeparator("=");
  console.log(`结果: ${pass}/${pass + fail} 通过`);

  return fail === 0 ? 0 : 1;
}

process.exit(main());
