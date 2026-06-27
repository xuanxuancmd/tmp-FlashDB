---
name: harness-dev-workflow
description: >-
  自主编码闭环 Skill。按 Plan 编排编码(TDD驱动)→评估→覆盖率门控→检视→修复→完成,支持单 Plan / 多 Plan(并发 worktree)两种模式。
---

# harness-dev-workflow

## 职责

Plan→编码(TDD驱动)→质量门禁→覆盖率门控→修复→完成的自主闭环。不执行需求分析或 Plan 生成(Plan 是输入)。

## TDD 原则

编码阶段强制 TDD（测试驱动开发）流程：
1. **先写测试**：根据 Plan 中的 BDD 场景和 C 源码函数签名，先编写 Rust 测试用例
2. **测试失败验证**：运行 `cargo test` 确认测试失败（因为实现尚未编写）
3. **再写实现**：编写最小实现使测试通过
4. **重构**：优化代码，确保测试仍然通过

每个 task 遵循：**红（测试失败）→ 绿（测试通过）→ 重构** 循环。

## 输入

### 必选(二选一)

| 参数 | 说明 | 示例 |
|------|------|------|
| `plan_list` | Plan 文件路径列表 | `[".sisyphus/plans/runtime-plan.md"]` |
| `requirement_text` | 一句话需求(当 plan_list 为空时必填) | "实现 offset 管理功能" |

### 可选

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `module_name` | 模块名(用于 state 命名) | 从 plan 文件名或 requirement 推断，项目级默认 `project` |

## 预检(Pre-flight)

每次启动时执行,任一失败 → BLOCKED:

1. **State 续传**:Read `.opencode/harness/state/{module}-workflow-state.json`;按 status 分支处理(详见状态恢复段)
2. **Plan 可用**:`plan_list` 为空 → 从 `requirement_text` 构建 plan_list;也为空 → BLOCKED

## 单 Plan 执行

符合`plan_list.length == 1` 为单plan。

### 流程


```
[编码(TDD) task 1..N] → [evaluating] → [coverage_check] → [reviewing] → [completing]
```
*此处`reviewing`等同于full_reviewing*

### 详情

- **编码(TDD驱动)**：加载以下技能。每个 task 按 TDD 流程：先写测试→验证失败→写实现→验证通过。每个 task 完成后运行 `cargo check && cargo test` 验证编译和测试通过，再刷新 state。

| 技能 | 用途 |
|------|------|
| c-translate-to-rust | C 到 Rust 1:1 代码翻译实战指南（翻译映射速查表/禁令表/易错表） |
| harness-bdd-coding | BDD 测试代码实现指导（Feature 拷贝、Cucumber 测试代码生成、Evidence 收集） |

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **coverage_check**:运行覆盖率检查，门控 ≥90%。失败 → Fixing（补充测试）。

  ```bash
  cargo llvm-cov --workspace --json | jq -r '.data[0].totals.lines.percent'
  # 如果 < 90 → 补充属性测试/边界测试 → 重新检查
  # 如果 ≥ 90 → 通过，进入 reviewing
  ```

  **覆盖率不足时的修复策略**：
  1. 读取 `cargo llvm-cov --summary` 报告，定位未覆盖的文件和行
  2. 对照 C 源码，识别未覆盖的分支（错误路径、边界条件）
  3. 补充 `#[test]` 或 `proptest!` 用例覆盖缺失路径
  4. 重新运行覆盖率检查
  5. 最多 3 次循环 → 仍不足 → `status = "blocked"` 上报人工

- **reviewing**(占位符为空则跳过此阶段): 加载以下技能。

| 技能 | 用途 |
|------|------|
| harness-translate-code-review | 翻译项目检视编排（通用检视 + 翻译忠实度检视并行） |
| harness-code-review | 全量代码检视（mode=full，所有 Plan 完成后执行） |

- evaluating、coverage_check 和 reviewing 任一阶段失败 → Fixing(见下)。

## 多 Plan 执行(强制 worktree 隔离)

`plan_list.length > 1` 时,每个 Plan 分配独立 git worktree + sub-agent。

### 流程

#### Phase 1: 并行编码 + 增量检视(每个 worktree 独立)

主 Agent 为每个 Plan 创建 worktree 并并行启动 sub-agent:

```
task(category="deep", load_skills=["c-translate-to-rust", "harness-bdd-coding"], run_in_background=true,
     workdir="worktrees/{plan_name}",
     prompt="Plan: {plan_path}, module: {module},
             state: .opencode/harness/state/{module}-{plan}-state.json")
```

每个 sub-agent 在自己 worktree 内执行:

```
coding(TDD) → evaluating → coverage_check → incremental_reviewing → completing (status=completed)
```

> sub-agent **不执行** full_reviewing(全量审查由主 Agent merge 后统一执行)。

#### Phase 2: 串行 merge

全部 sub-agent 完成后,主 Agent 按 plan_list 顺序逐个 merge:

- 读 worktree 内 plan state → 确认 `status="completed"`
- `git merge feature/{plan}` → 刷新主 state `plan_status[plan]="completed"`
- `git worktree remove worktrees/{plan}`

> merge 是 git 操作,不是 workflow phase。

#### Phase 3: 全局检视(主干)

主 Agent 在主干执行:

```
evaluating → coverage_check → full_reviewing → completing
```

- evaluating: `harness-code-evaluator` skill(跨 Plan 整体评估)
- full_reviewing(为空则跳过): 加载 full_review_skills

### 详情

- **编码(TDD驱动)**：加载以下技能。每个 task 按 TDD 流程：先写测试→验证失败→写实现→验证通过。每个 task 完成后运行 `cargo check && cargo test` 验证编译和测试通过，再刷新 state。

| 技能 | 用途 |
|------|------|
| c-translate-to-rust | C 到 Rust 1:1 代码翻译实战指南（翻译映射速查表/禁令表/易错表） |
| harness-bdd-coding | BDD 测试代码实现指导（Feature 拷贝、Cucumber 测试代码生成、Evidence 收集） |

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **coverage_check**:运行覆盖率检查，门控 ≥90%。失败 → Fixing（补充测试）。

- **incremental_reviewing**(每 Plan 完成后执行): 调用 code-review-agent 执行增量检视。

```
task(
  subagent_type="code-review-agent",
  description="增量代码检视",
  load_skills=["harness-code-review"],
  run_in_background=false,
  prompt="mode: incremental
module: {module_name}
plan: {plan_path}
state: .opencode/harness/state/{module_name}-workflow-state.json
执行增量代码检视（每 Plan 完成后）"
)
```

- **full_reviewing**(所有 Plan 完成后执行): 加载以下技能。

| 技能 | 用途 |
| harness-translate-code-review | 翻译项目检视编排（通用检视 + 翻译忠实度检视并行） |
| harness-code-review | 全量代码检视（mode=full，所有 Plan 完成后执行） |

- evaluating、coverage_check、incremental_reviewing、full_reviewing 任一阶段失败 → Fixing(见下)。

> Sub-agent BLOCKED 时主 Agent 通过 `question()` 询问:**跳过该 Plan / 主 Agent 进入修复 / 全部中止**。

## Fixing 阶段

任何 evaluating / coverage_check / incremental_reviewing / full_reviewing 失败时进入:

1. **读报告**提取 HIGH issue
2. **记录修复尝试**:`attempt_counts[scope_key]` 递增 + `error_signature` + `strategies_tried`
   - `scope_key`:state 的 `plan` 字段非空则为该 plan 路径;否则 `"global"`
3. **修复代码**(每次必须换策略;禁相同代码重试)
4. **重跑失败阶段**,刷新 state: `phase = trigger_stage`, `fixing = null`
5. Max 3 次 → `status = "blocked"` 上报人工

## State 管理

**所有 state.json 使用统一 schema**,字段按角色(执行 vs 编排)填充 null/非 null。

### 统一 Schema

```json
{
  "module": "{module_name}",
  "plan": "...plan-a.md | null",
  "truth_source_path": ["...plan-a.md"],
  "status": "running | blocked | completed",
  "last_run": "2026-06-26T10:30:00Z",
  "workflow": {
    "phase": "coding | evaluating | coverage_check | incremental_reviewing | full_reviewing | fixing | completing",
    "current_plan": "...plan-a.md | null",
    "current_task": "... | null",
    "current_skill": "... | null",
    "fixing": null | {"trigger_stage": "evaluating | incremental_reviewing | full_reviewing", "reports": [...]},
    "tasks_completed": ["task_001"],
    "tasks_remaining": ["task_003"],
    "plan_status": null | {
      "...plan-a.md": "pending | running | completed | blocked"
    },
    "attempt_counts": {
      "...plan-a.md | global": {"count": 0, "error_signature": null, "strategies_tried": []}
    }
  }
}
```

### 三种 state 文件的角色

| state 文件 | 路径 | `plan` 字段 | `plan_status` 字段 | 写入者 |
|-----------|------|-------------|-------------------|--------|
| 单 Plan 主 state | `.opencode/harness/state/{module}-workflow-state.json` | 唯一 plan 路径 | null | 主 Agent |
| 多 Plan 主 state | 同上 | null | 含所有 plan 的 map | 主 Agent |
| 多 Plan 执行 state | `worktrees/{plan}/.opencode/harness/state/{module}-{plan}-state.json` | 当前 plan 路径 | null | sub-agent |

### 字段语义

| 字段 | 取值 | 说明 |
|------|------|------|
| `plan` | 路径/null | 当前 state 归属的 Plan;编排 state 为 null |
| `plan_status` | null/map | 仅多 Plan 主 state 非空,记录每个 plan 的编排进度 |
| `current_plan` | 路径/null | 正在执行的 Plan;编排 state / Global 阶段为 null |
| `current_task` | task_id/null | 编码中的 task;非编码阶段为 null |
| `fixing.trigger_stage` | evaluating/incremental_reviewing/full_reviewing | 触发 fixing 的阶段 |
| `fixing.reports` | 路径列表 | 失败报告路径 |
| `attempt_counts[key]` | plan 路径/"global" | 对应 scope 的修复次数记录 |

## Phase 状态机与 State 刷新时机

**每个 phase 的进入/退出都必须刷新 state.json。下表是唯一的刷新规则来源,其他段落不再重复。**

| Phase | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | plan 启动 / 上一 task 完成 | 全部完成 → evaluating | phase="coding" | 每 task: tasks_completed += / tasks_remaining -= / current_task = next |
| `evaluating` | coding 完成 或 merge 完成(多 plan) | 通过 → coverage_check;失败 → fixing | phase="evaluating", current_skill="evaluator" | 通过: current_skill=null; 失败: phase="fixing" + fixing={trigger_stage, reports} + attempt_counts++ |
| `coverage_check` | evaluating 通过 | 通过(≥90%) → incremental_reviewing(单 plan/worktree) 或 full_reviewing(多 plan 主干);失败(<90%) → fixing | phase="coverage_check", current_skill="llvm-cov" | 通过: current_skill=null; 失败: phase="fixing" + fixing={trigger_stage:"coverage_check", reports} + attempt_counts++ |
| `incremental_reviewing` | coverage_check 通过 | 通过 → full_reviewing(单 plan) 或 completing(worktree);失败 → fixing | phase="incremental_reviewing" | 通过: current_skill=null; 失败: phase="fixing" + fixing={...} + attempt_counts++ |
| `full_reviewing` | incremental_reviewing 通过(单 plan) 或 merge 完成(多 plan 主干) | 通过 → completing;失败 → fixing | phase="full_reviewing" | 通过: current_skill=null; 失败: phase="fixing" + fixing={...} + attempt_counts++ |
| `fixing` | 任一 fail | 修完 → 返回 trigger_stage;超限 → blocked | phase="fixing", fixing={...} | 修完: phase=trigger_stage, fixing=null; 超限: status="blocked", blocked_reason={...} |
| `completing` | full_reviewing 通过 或 incremental_reviewing 通过(worktree 无 full) | status="completed" | phase="completing", current_plan=null, current_task=null, current_skill=null | status="completed" |

> `fixing.trigger_stage` 可选值: `evaluating | coverage_check | incremental_reviewing | full_reviewing`

### 多 Plan 编排阶段的额外注意事项

| 时机 | 写入字段 |
|------|---------|
| 创建 worktree + 启动 sub-agent | `plan_status[plan]="running"` |
| Sub-agent 完成(读其 plan state 后) | `plan_status[plan]="completed"` 或 `"blocked"` |
| 全部 plan_status="completed" 后 | 主 Agent 串行 git merge,逐个将 `plan_status[plan]="completed"`(merge 成功后保持);全部完成后进入 evaluating |

> **核心约束**:AI 不调 Write 工具,state.json 就不会更新。续传完全依赖 state 文件,上表每个时机都必须刷新。

### 状态恢复

1. **Read state.json**:不存在 → 全新启动,按 `plan_list.length` 决定角色(单 Plan:执行;多 Plan:编排)
2. **识别角色**:`plan_status` 是否为 null → 执行状态(false) / 编排状态(true)
3. **status 分支**:`blocked` → `question()` 上报;`completed` → `question("重启?")`
4. **按 phase 恢复**:
   - 执行状态:`phase + current_plan + current_task` → 续传该 task 或进入相应 phase
   - 编排状态:`plan_status` 中 status="running" 的 plan → 重启对应 sub-agent;全部 completed → 进入 evaluating

## 禁止事项

1. ❌ 修改 Plan / 测试文件来"通过"(含删除失败测试)
2. ❌ 不读错误输出就重试,或用相同代码重试
3. ❌ Task 级执行 evaluating/incremental_reviewing/full_reviewing/fixing(只在 phase 层级)
4. ❌ AI 自评替代门禁(必须执行命令或委托 sub-agent)
5. ❌ 漏刷新 state.json(上表每个时机必须全部执行)
6. ❌ 多 Plan 主 Agent 直接编码(必须 sub-agent + worktree)
7. ❌ 跳过 worktree(每个 Plan 必须独立 worktree,禁共享主干)
8. ❌ 并发 merge 到主干(必须按 plan_list 顺序串行)
9. ❌ Sub-agent 写主 state.json(只写自己 worktree 内的 plan state)
10. ❌ Merge 前未确认 plan state.json 的 status="completed"
11. ❌ 修改 `truth_source_path` 中的 Plan 路径数组
