# Phase 状态机与 State 刷新时机

**state.json 的 phase 字段在主 Agent 和 executor 各自的切换处刷新。**
主 Agent 负责编排阶段(reviewing / evaluating / fixing / completing);executor 只负责执行阶段(coding / fixing)。

## Sub-agent（code-executor-agent）状态机

| Phase | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | 主 Agent 派发 executor(mode=coding) | 全部 task 完成（build+test 均 pass） | `phase="coding"`, `status="running"`, `mode="coding"` | 每 task: `tasks_completed += [id]`, `tasks_remaining -= [id]`;全部完成后: `status="completed"` |
| `fixing` | 主 Agent 派发 executor(mode=fix) | 全部 issue 修复完成（build+test 均 pass） | `phase="fixing"`, `status="running"`, `mode="fix"` | 每 issue: `tasks_completed += [id]`, `tasks_remaining -= [id]`;全部完成后: `status="completed"` |

executor 不进入 reviewing / evaluating 阶段。build+test 自检失败时按 deviation rule 在 coding/fixing 阶段内部修复（每 task/issue 最多 3 次），超过则 `status="blocked"`。

## Workflow（主 Agent）状态机

| Phase | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | 单 Plan: 派发 executor(mode=coding);多 Plan: 串行或并发派发多个 executor(mode=coding) | 所有 executor 返回 status=completed | `phase="coding"`, `plan_status[plan]="running"` | 全部 executor `status="completed"` 后: `phase="reviewing"` |
| `reviewing` | coding 完成 | 通过 → `evaluating`；失败 → `fixing` | `phase="reviewing"` | 通过: `current_skill=null`；失败: `phase="fixing"` + `fixing={trigger_stage:"reviewing", reports}` + `attempt_counts++` |
| `evaluating` | reviewing 通过 | 通过 → `completing`；失败 → `fixing` | `phase="evaluating"`, `current_skill="evaluator"` | 通过: `current_skill=null`；失败: `phase="fixing"` + `fixing={trigger_stage:"evaluating", reports}` + `attempt_counts++` |
| `fixing` | reviewing 或 evaluating 失败 | 修完 → 回到 trigger_stage；超限 → blocked | `phase="fixing"`, `fixing={...}` | 修完: `phase={trigger_stage}`, `fixing=null`；超限: `status="blocked"` |
| `completing` | evaluating 通过 | `status="completed"` | `phase="completing"`, `current_plan=null`, `current_task=null`, `current_skill=null` | `status="completed"` |

### 多 Plan 编排阶段的额外注意事项

多 Plan 场景涉及三层状态：主 state（编排进度）+ 各 executor state（执行进度）+ worktree（代码隔离）。三者必须保持一致。

| 时机 | 主 state 写入 | executor state 写入 | worktree 操作 |
|------|-------------|---------------------|--------------|
| 创建 worktree + 派发 executor(mode=coding) | `plan_status[plan]="running"` | executor 初始化: `status="running"`, `mode="coding"`, `phase="coding"` | `git worktree add` |
| executor 每完成一个 task | — | `tasks_completed += [id]`, `tasks_remaining -= [id]`, `current_task=next` | — |
| executor 完成（主 Agent 读其 state 后） | `plan_status[plan]="completed"` 或 `"blocked"` | `status="completed"` 或 `"blocked"` | — |
| 派发 executor(mode=fix) 修复 | `fixing={trigger_stage, reports}` | executor: `status="running"`, `mode="fix"`, `phase="fixing"` | — |
| 全部 plan_status="completed" 后 | `phase="reviewing"` | — | — |
| Plan merge 到 main 后 | `plan_status[plan]="merged"` | — | — |
| worktree 清理后 | — | — | `git worktree remove` + `git branch -d` |

#### 断点续传的一致性保证

续传时主 Agent 必须核对三层状态的一致性：

| 不一致情况 | 处理 |
|-----------|------|
| `plan_status[plan]="running"` 但 executor state 不存在 | executor 从未启动 → 正常派发 executor(mode=coding) |
| `plan_status[plan]="running"` 且 executor state `status="completed"` | executor 已完成但主 state 未更新 → 更新 `plan_status[plan]="completed"` |
| `plan_status[plan]="completed"` 但 worktree 不存在 | worktree 丢失 → 检查分支：存在则重建 worktree；不存在则该 Plan 需重新编码 |
| `plan_status[plan]="merged"` 但 worktree 仍存在 | worktree 未清理 → `git worktree remove` 清理 |

> **核心约束**:AI 不调 Write 工具,state.json 就不会更新。续传完全依赖 state 文件,上表每个时机都必须刷新。
