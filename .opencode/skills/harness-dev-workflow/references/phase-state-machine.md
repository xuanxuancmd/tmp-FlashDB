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
