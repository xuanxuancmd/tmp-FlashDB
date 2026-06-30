# Workflow Skill 模板

> 生成自主编码闭环 Skill 时,按此模板填充。`{变量}` 由 meta skill 替换为项目实际值。

---

## 生成的文件结构

```
.opencode/skills/{skill_name}/
├── SKILL.md                              # 薄入口（≤80 行）
└── references/
    ├── workflow-single-plan.md           # 单 Plan 执行流程（含 Step 0/HARD GATE/Self-Check/success_criteria）
    ├── workflow-multi-plan.md            # 多 Plan 并行编排（含主Agent+Sub-agent 双层 todo/GATE）
    ├── fixing-loop.md                    # Fixing 阶段 + 策略去重 + attempt_counts
    ├── phase-state-machine.md            # Phase 状态机表 + 刷新时机表
    └── state-schema.md                   # 统一 state.json schema + 三种 state 角色
```

---

## SKILL.md 模板（薄入口）

```markdown
---
name: {skill_name}
description: >-
  自主编码闭环 Skill。按 Plan 编排编码→评估→检视→修复→完成,支持单 Plan / 多 Plan(并发 sub-agent)两种模式。
---

# {skill_name}

## 职责

Plan→编码→质量门禁→修复→完成的自主闭环。不执行需求分析或 Plan 生成(Plan 是输入)。

## 全自动化约束（核心原则）

本 workflow 全自动执行,**唯一暂停例外是显式 `question()` 调用**。

- ✅ **允许暂停**:workflow 内显式 `question()`(当前 2 处:Sub-agent BLOCKED 决策、状态恢复 blocked/completed 决策)
- ❌ **禁止**:Agent 自主暂停(如"等待用户回应"、"上报后停止"、非 `question()` 的文字询问)
- ❌ **禁止**:隐式请求介入(如"向用户报告后等待"、"请求用户决策"等非 `question()` 表述)
- ✅ **替代方式**:需要人工介入但不满足 `question()` 场景时,用异步通道(写报告文件 + 继续执行/终止,不等待)

> sub-skill 内部的 `question()`(如 CatA 审批、evaluator 3 次失败)由 sub-skill 自行定义,workflow 不重复硬编码。

## 输入

### 必选(二选一)

| 参数 | 说明 | 示例 |
|------|------|------|
| `plan_list` | Plan 文件路径列表 | `[".sisyphus/plans/runtime-plan.md"]` |
| `requirement_text` | 一句话需求(当 plan_list 为空时必填) | "实现 offset 管理功能" |

### 可选

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `module_name` | 模块名(用于 state 命名) | 从 plan 文件名或 requirement 推断,项目级默认 `project` |

## 预检(Pre-flight)

每次启动时执行,任一失败 → BLOCKED:

1. **State 续传**:Read `.opencode/harness/state/{module}-workflow-state.json`;按 status 分支处理(详见 `references/state-schema.md` 状态恢复段)
2. **Plan 可用**:`plan_list` 为空 → 从 `requirement_text` 构建 plan_list;也为空 → BLOCKED

## 执行模式判定

- `plan_list.length == 1` → 单 Plan 执行,加载 `references/workflow-single-plan.md`
- `plan_list.length > 1` → 多 Plan 执行(强制并行 sub-agent,每个 Plan 独立 code-executor agent),加载 `references/workflow-multi-plan.md`

## 执行上下文

@./references/workflow-single-plan.md
@./references/workflow-multi-plan.md
@./references/fixing-loop.md
@./references/phase-state-machine.md
@./references/state-schema.md

## 禁止事项

1. ❌ 修改 Plan / 测试文件来"通过"(含删除失败测试)
2. ❌ 不读错误输出就重试,或用相同代码重试
3. ❌ Task 级执行 evaluating/incremental_reviewing/full_reviewing/fixing(只在 phase 层级)
4. ❌ AI 自评替代确定性门禁(必须执行命令或委托 sub-agent)
5. ❌ 漏刷新 state.json(每个时机必须全部执行)
6. ❌ 多 Plan 主 Agent 直接编码(必须用 `subagent_type="code-executor-agent"` + `background=true` 并行派发)
7. ❌ 使用 oh-my-openagent plugin 的 `run_in_background` / `background_output` / `background_cancel` 工具(OpenCode 原生 `task(background=true)` 已内置完成通知机制)
8. ❌ 有文件修改冲突的 Plan 并发派发(主 Agent 必须分析每个 Plan 的 `files_modified`，将修改相同文件的 Plan 串行化)
9. ❌ Sub-agent 写主 Agent 的 state.json(只写各自 plan 对应的 state_path)
10. ❌ 未确认各 plan state.json 的 status="completed" 就进入 Phase 3
11. ❌ 修改 `truth_source_path` 中的 Plan 路径数组
12. ❌ Agent 自主暂停(非显式 `question()` 的任何暂停行为)
13. ❌ 隐式请求介入(用文字描述"等待用户"等代替 `question()` 调用)
```

---

## references/workflow-single-plan.md 模板

```markdown
# 单 Plan 执行 Workflow

## 流程

```
[Step 0 清单初始化] → [编码 task 1..N] → [evaluating] → [reviewing] → [completing]
```
*此处`reviewing`等同于full_reviewing*

## Step 0: 流程清单初始化（强制执行，不可跳过）

在开始任何编码工作前，**必须**调用 TodoWrite 创建完整流程清单。跳过此步骤视为流程违规。

### todo 模板

```
1. ☐ 编码：实现 Plan 中所有 task
2. ☐ 对抗性评估（evaluating）：调用 harness-code-evaluator
3. ☐ 代码检视（reviewing）：加载检视技能执行（占位符为空则跳过）
4. ☐ 完成（completing）：刷新 state status=completed + Self-Check
```

> TodoWrite 让阶段跳过变得"可见"。后续每个阶段完成时必须将对应项标记为 completed，completing 阶段会回溯校验。

## 阶段详情

- **编码**:加载以下技能:

  {coding_skills}

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **reviewing**(占位符为空则跳过此阶段): 加载以下技能:

  {full_review_skills}

- evaluating和reviewing 任一阶段失败 → Fixing(见 `fixing-loop.md`)。

## HARD GATE — 阶段切换验证

每个阶段切换处必须通过以下验证，未通过则 BLOCKED，不得进入下一阶段。**这不是建议，未通过门禁的阶段切换是非法的。**

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| 编码 → evaluating | ① Plan 中所有 task 已实现 ② state.json 已刷新 phase="evaluating" ③ TodoWrite "编码"项已 completed | 任一未满足 → BLOCKED，返回编码阶段 |
| evaluating → reviewing | ① evaluating 报告已生成（.opencode/harness/evidence/ 下有对应文件）② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "evaluating"项已 completed | 任一未满足 → BLOCKED，返回 evaluating 阶段 |
| reviewing → completing | ① reviewing 报告已生成（或占位符为空已记录跳过）② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "reviewing"项已 completed | 任一未满足 → BLOCKED，返回 reviewing 阶段 |

## completing 阶段 Self-Check

在刷新 state status=completed 前，**回溯验证**所有阶段已实际执行：

1. Read state.json → 确认 phase 曾经过 evaluating（current_skill 历史含 "evaluator"）
2. 确认 evaluating 报告存在（`.opencode/harness/evidence/harness-dev-workflow-evaluator-review.json`）
3. 确认 reviewing 报告存在（或占位符为空已记录跳过）
4. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## success_criteria

- [ ] Plan 中所有 task 已编码实现
- [ ] evaluating 阶段已执行（harness-code-evaluator 已调用）
- [ ] reviewing 阶段已执行（或占位符为空已跳过）
- [ ] state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（所有阶段报告存在）
```

---

## references/workflow-multi-plan.md 模板

```markdown
# 多 Plan 执行 Workflow（强制并行 code-executor sub-agent）

`plan_list.length > 1` 时，强制为每个 Plan 派一个独立的 `code-executor` sub-agent（`mode: subagent`，全新上下文）。不依赖 oh-my-opencode plugin 的 `category` 路由，使用 OpenCode 原生 `task` 工具的 `subagent_type` + `background=true` 参数（Go 二进制内置能力，无需 plugin）。

## 总流程

```
[主 Agent Step 0 编排清单] 
  → Phase 1: 并行派发 code-executor sub-agent（每个 sub-agent 独立上下文，background=true）
  → Phase 2: 等待所有 sub-agent 完成，汇总结果
  → Phase 3: 主 Agent 全局检视
  → [主 Agent completing + Self-Check]
```

## 主 Agent Step 0: 编排清单初始化（强制执行，不可跳过）

主 Agent 在启动任何 sub-agent 前，**必须**调用 TodoWrite 创建编排清单。跳过此步骤视为流程违规。

### 主 Agent todo 模板

```
1. ☐ Phase 1: 并行派发每个 Plan 的 code-executor sub-agent（subagent_type + background=true）
2. ☐ Phase 1: 等待所有 sub-agent 完成（读各 plan state）
3. ☐ Phase 2: 汇总各 plan 结果（检查 status，处理 blocked）
4. ☐ Phase 3 evaluating: 调用 harness-code-evaluator（跨 Plan 整体评估）
5. ☐ Phase 3 full_reviewing: 加载检视技能执行（占位符为空则跳过）
6. ☐ completing: 刷新主 state status=completed + Self-Check
```

## Phase 1: 并行编码 + 增量检视(每个 sub-agent 独立上下文)

主 Agent 为每个 Plan 并行派发独立的 `code-executor` sub-agent。**不依赖 `category` 路由**（oh-my-opencode plugin），使用 OpenCode 原生 `task` 工具的 `subagent_type` + `background=true` 参数（Go 二进制内置能力，无需 plugin）。每个 sub-agent 拥有全新上下文（OpenCode 天然 session 隔离），互不干扰。

```
task(subagent_type="code-executor-agent", background=true,
     description="Execute plan {plan_name}",
     prompt="""
       plan_path: {plan_path}
       module_name: {module}
       state_path: .opencode/harness/state/{module}-{plan}-state.json
     """)
```

### Sub-agent 子流程（每个 code-executor 独立执行编码，由 agent .md 定义）

**executor 只负责编码 + 测试 + 自检，不执行任何 evaluating / reviewing。** 每个 sub-agent 在自己的独立 session 中执行：

```
[Step 0 清单初始化] → coding → completing (status=completed, build+test 全部通过)
```

> **职责边界**：
> - ✅ 实现 Plan 中的每个 task
> - ✅ 为每个 task 编写/维护测试（TDD 或后置）
> - ✅ 每个 task 完成后执行 `{build_cmd}` + `{test_cmd}` 自检
> - ✅ 编码过程中的 deviation 自动修复（每个 task 最多 3 次）
> - ✅ 创建 Summary（完成状态 + 偏差记录）
> - ❌ 不调用 harness-code-evaluator 或任何 review agent
> - ❌ 不执行 Fixing 子流程（Fixing 由 workflow 层统一负责）
> - ❌ 不刷新主 Agent 的 state.json

#### Sub-agent Step 0: 流程清单初始化（强制执行，不可跳过）

每个 sub-agent 在编码前，**必须**调用 TodoWrite 创建流程清单。跳过此步骤视为流程违规。

**sub-agent todo 模板**:

```
1. ☐ 编码：实现当前 Plan 的所有 task（每 task 后自检 build+test）
2. ☐ 完成（completing）：刷新 plan state status=completed + Self-Check
```

#### Sub-agent HARD GATE — coding 完成验证

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| coding → completing | ① Plan 中所有 task 已实现 ② 所有 task 的 `{build_cmd}` + `{test_cmd}` 均已通过 ③ plan state.json 的 `tasks_remaining` 为空数组 ④ TodoWrite "编码"项已 completed ⑤ 每个 task 的 commit 已记录 | 任一未满足 → BLOCKED，返回编码阶段 |

#### Sub-agent completing Self-Check

在刷新 plan state status=completed 前，**回溯验证**编码阶段已实际执行：

1. Read plan state.json → `tasks_completed` 包含 Plan 中所有 task_id
2. `tasks_remaining` 为空数组
3. 最近一次 `{build_cmd}` / `{test_cmd}` 执行结果均 pass
4. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到编码阶段重新执行，不标记 completed。**

#### Sub-agent success_criteria

- [ ] 当前 Plan 中所有 task 已编码实现并提交
- [ ] 所有 task 的 `{build_cmd}` + `{test_cmd}` 均已通过
- [ ] plan state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（tasks 全部完成，无遗留阻塞）

## Phase 2: 等待 + 状态汇总（并行 sub-agent 全部完成后）

所有 `background=true` 派发的 sub-agent 完成后，主 Agent 等待原生 `<task id="..." state="completed">` XML 通知（由 OpenCode 自动注入会话），逐个读 plan state 文件：

- 无需 `background_output`（原生机制自动返回 sub-agent 完整输出）
- Read 各 plan 的 `state_path` → 确认 `status="completed"`
- 任一 plan status="blocked" → 主 Agent 汇总 blocked-reports，进入全局 fixing 或直接终止

## Phase 3: 全局检视(主 Agent 直接执行)

主 Agent 在主干执行:

```
evaluating → full_reviewing → completing
```

- evaluating: `harness-code-evaluator` skill(跨 Plan 整体评估)
- full_reviewing(占位符为空则跳过):

## 阶段详情

- **编码**:加载以下技能:

  {coding_skills}

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **incremental_reviewing**(占位符为空则跳过此阶段):

  {incremental_review_skills}

- **full_reviewing**(占位符为空则跳过此阶段):加载以下技能:

  {full_review_skills}

- evaluating、incremental_reviewing、full_reviewing 任一阶段失败 → Fixing(见 `fixing-loop.md`)。

## 主 Agent HARD GATE — 全局阶段切换验证

主 Agent 在 Phase 3 全局检视的阶段切换处必须通过验证，未通过则 BLOCKED。**这不是建议，未通过门禁的阶段切换是非法的。**

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| Phase 2 汇总完成 → Phase 3 evaluating | ① 所有 plan_status="completed" ② 主 state 已刷新 phase="evaluating" ③ TodoWrite "Phase 2 汇总"项已 completed | 任一未满足 → BLOCKED |
| Phase 3 evaluating → full_reviewing | ① evaluating 报告已生成 ② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "Phase 3 evaluating"项已 completed | 任一未满足 → BLOCKED，返回 evaluating |
| Phase 3 full_reviewing → completing | ① full_reviewing 报告已生成（或占位符为空已记录跳过）② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "Phase 3 full_reviewing"项已 completed | 任一未满足 → BLOCKED，返回 full_reviewing |

## 主 Agent completing Self-Check

在刷新主 state status=completed 前，**回溯验证**所有阶段已实际执行：

1. Read 主 state.json → 确认所有 plan_status="completed"
2. 确认 Phase 3 evaluating 报告存在
3. 确认 Phase 3 full_reviewing 报告存在（或占位符为空已记录跳过）
4. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## 主 Agent success_criteria

- [ ] 所有 Plan 的 sub-agent 已完成（plan_status 全部 completed）
- [ ] 所有 plan state.json 均已读回，status="completed"
- [ ] Phase 3 evaluating 阶段已执行（harness-code-evaluator 已调用）
- [ ] Phase 3 full_reviewing 阶段已执行（或占位符为空已跳过）
- [ ] 主 state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（所有阶段报告存在）
```

---

## references/fixing-loop.md 模板

```markdown
# Fixing 阶段

任何 evaluating / incremental_reviewing / full_reviewing 失败时进入:

1. **读报告**提取 HIGH issue
2. **记录修复尝试**:`attempt_counts[scope_key]` 递增 + `error_signature` + `strategies_tried`
   - `scope_key`:state 的 `plan` 字段非空则为该 plan 路径;否则 `"global"`
3. **加载 fix-self-check skill（若存在）**执行修前检查:因果链诊断 + 爆炸半径论证 + 策略验证。不通过 → 再思考（回退/换策略/拆分），不人工
4. **修复代码**(每次必须换策略;禁相同代码重试)
5. **加载 fix-self-check skill（若存在）**执行修后检查:回归检测 + diff 质量 + 健康趋势。不通过 → 最小粒度回退 + 再思考
6. **重跑失败阶段**,刷新 state: `phase = trigger_stage`, `fixing = null`
7. Max 5 次 → `status = "blocked"`,写入 `.opencode/harness/blocked-reports/{module}-{timestamp}.md`(含:失败阶段、已试策略列表、error_signature、剩余 HIGH issues、建议的人工介入方向),workflow 终止退出

> BLOCKED 终止不调 `question()` 等待。用户审阅 blocked-reports 后用显式命令 resume。
> fix-self-check 是修复思维方式的门禁,不替代 retry 计数和 state 管理(那些由本 skill 负责)。fix-self-check 只管 agent 怎么想,不管怎么计数。
```

---

## references/phase-state-machine.md 模板

```markdown
# Phase 状态机与 State 刷新时机

**state.json 的 phase 字段仅在对应执行者（executor / workflow）的切换处刷新。**
executor（code-executor-agent）只进入 `coding` phase；evaluating / reviewing / fixing / completing 全部由 workflow skill（单 Plan / 多 Plan 主 Agent）负责。

## Sub-agent（code-executor-agent）状态机

| Phase | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | Plan 派发时启动 | 全部 task 完成（build+test 均 pass） | `phase="coding"`, `status="running"` | 每 task: `tasks_completed += [id]`, `tasks_remaining -= [id]`, `current_task` 更新；全部完成后: `status="completed"` |

build+test 自检失败时 coding 阶段内部修复（每 task 最多 3 次），超过则 `status="blocked"`。

## Workflow（单 Plan / 多 Plan 主 Agent）状态机

| Phase | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | 单 Plan: workflow 直接执行；多 Plan: 仅由 executor sub-agent 执行（主 Agent 不进入） | 单 Plan: 全部 task 完成；多 Plan: 全部 sub-agent 完成 | `phase="coding"` | 单 Plan: 每 task 后刷新；多 Plan: 全部 sub-agent `status="completed"` 后退出 |
| `evaluating` | coding 完成（单 Plan 自身 / 多 Plan 全部 sub-agent 完成） | 通过 → `incremental_reviewing`（有技能）或 `full_reviewing`；失败 → `fixing` | `phase="evaluating"`, `current_skill="evaluator"` | 通过: `current_skill=null`；失败: `phase="fixing"` + `fixing={trigger_stage, reports}` + `attempt_counts++` |
| `incremental_reviewing` | evaluating 通过 | 通过 → `full_reviewing`；失败 → `fixing` | `phase="incremental_reviewing"` | 通过: `current_skill=null`；失败: `phase="fixing"` + `fixing={...}` + `attempt_counts++` |
| `full_reviewing` | incremental_reviewing 通过（有技能）或 evaluating 直接通过（无增量技能） | 通过 → `completing`；失败 → `fixing` | `phase="full_reviewing"` | 通过: `current_skill=null`；失败: `phase="fixing"` + `fixing={...}` + `attempt_counts++` |
| `fixing` | 任一 fail | 修完 → 回到 trigger_stage；超限 → blocked | `phase="fixing"`, `fixing={...}` | 修完: `phase={trigger_stage}`, `fixing=null`；超限: `status="blocked"` |
| `completing` | full_reviewing 通过（或无 full_reviewing 技能时 incremental_reviewing 通过） | `status="completed"` | `phase="completing"`, `current_plan=null`, `current_task=null`, `current_skill=null` | `status="completed"` |

### 多 Plan 编排阶段的额外注意事项

| 时机 | 写入字段 |
|------|---------|
| 派发 sub-agent（subagent_type + background=true） | `plan_status[plan]="running"` |
| Sub-agent 完成（读其 plan state 后） | `plan_status[plan]="completed"` 或 `"blocked"` |
| 全部 plan_status="completed" 后 | 主 Agent 进入 evaluating 阶段 |

> **核心约束**:AI 不调 Write 工具,state.json 就不会更新。续传完全依赖 state 文件,上表每个时机都必须刷新。
```

---

## references/state-schema.md 模板

```markdown
# State 管理

**所有 state.json 使用统一 schema**,字段按角色(执行 vs 编排)填充 null/非 null。

## 统一 Schema

```json
{
  "module": "{module_name}",
  "plan": "...plan-a.md | null",
  "truth_source_path": ["...plan-a.md"],
  "status": "running | blocked | completed",
  "last_run": "2026-06-26T10:30:00Z",
  "workflow": {
    "phase": "coding | evaluating | incremental_reviewing | full_reviewing | fixing | completing",
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

## 三种 state 文件的角色

| state 文件 | 路径 | `plan` 字段 | `plan_status` 字段 | 写入者 |
|-----------|------|-------------|-------------------|--------|
| 单 Plan 主 state | `.opencode/harness/state/{module}-workflow-state.json` | 唯一 plan 路径 | null | 主 Agent |
| 多 Plan 主 state | 同上 | null | 含所有 plan 的 map | 主 Agent |
| 多 Plan 执行 state | `.opencode/harness/state/{module}-{plan}-state.json` | 当前 plan 路径 | null | sub-agent |

## 字段语义

| 字段 | 取值 | 说明 |
|------|------|------|
| `plan` | 路径/null | 当前 state 归属的 Plan;编排 state 为 null |
| `plan_status` | null/map | 仅多 Plan 主 state 非空,记录每个 plan 的编排进度 |
| `current_plan` | 路径/null | 正在执行的 Plan;编排 state / Global 阶段为 null |
| `current_task` | task_id/null | 编码中的 task;非编码阶段为 null |
| `fixing.trigger_stage` | evaluating/incremental_reviewing/full_reviewing | 触发 fixing 的阶段 |
| `fixing.reports` | 路径列表 | 失败报告路径 |
| `attempt_counts[key]` | plan 路径/"global" | 对应 scope 的修复次数记录 |

## 状态恢复

1. **Read state.json**:不存在 → 全新启动,按 `plan_list.length` 决定角色(单 Plan:执行;多 Plan:编排)
2. **识别角色**:`plan_status` 是否为 null → 执行状态(false) / 编排状态(true)
3. **status 分支**:`blocked` → `question()` 上报;`completed` → `question("重启?")`
4. **按 phase 恢复**:
   - 执行状态:`phase + current_plan + current_task` → 续传该 task 或进入相应 phase
   - 编排状态:`plan_status` 中 status="running" 的 plan → 重启对应 sub-agent;全部 completed → 进入 evaluating
```

---

## 模板变量说明

| 变量 | 来源 | 注入位置 | 示例值 |
|------|------|---------|--------|
| `{skill_name}` | 用户输入 | SKILL.md frontmatter + 标题 + evaluator 引用 | `harness-dev-workflow` |
| `{module_name}` | 运行时参数 | state schema 注释 | `connect-runtime` |
| `{coding_skills}` | Step 4.1 coding 阶段 | workflow-single-plan.md + workflow-multi-plan.md 的编码段 | markdown 表格 |
| `{incremental_review_skills}` | Step 4.1 incremental_reviewing 阶段 | workflow-multi-plan.md 的 incremental_reviewing 段 | task() 调用代码块 |
| `{full_review_skills}` | Step 4.1 full_reviewing 阶段 | workflow-single-plan.md + workflow-multi-plan.md 的 reviewing 段 | markdown 表格 |
| `{fixing_skills}` | Step 4.1 fixing 阶段 | fixing-loop.md（可选，若为空则 Fixing 不加载自检） | markdown 表格 |

## 生成规则

1. SKILL.md 写入 `.opencode/skills/{skill_name}/SKILL.md`（OpenCode）或 `.claude/skills/{skill_name}/SKILL.md`（Claude Code）
2. references/ 下 5 个文件写入 `.opencode/skills/{skill_name}/references/`（或 `.claude/skills/{skill_name}/references/`）
3. SKILL.md 行数 ≤80 行（薄入口）
4. 各 references 文件行数无硬性限制，但建议单个文件 ≤180 行
5. 占位符替换：meta skill 在 Step 6 生成时**必须完成所有 {变量} 替换**，生成的文件不含未替换占位符
6. 若某阶段 Skill 列表为空（如项目无 E2E Skill），对应占位符替换为"无（此阶段跳过）"，不删除行
