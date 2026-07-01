# State 管理

本文件是 state.json 的单一权威定义：Schema + stage 流转 + 状态机刷新时机 + 多 Plan 三层状态一致性 + 断点续传恢复。

## Schema

主 state 与子 plan state 使用同一 schema，主 state 额外含 `plan_status`（多 Plan 场景）。

```json
{
  "truth_source": "...plan-a.md | ...issue-report.md",
  "stage": "coding | reviewing | reviewed | evaluating | evaluated | fixing | stage_completed | completed | blocked",
  "current_agent": "code-executor-agent | code-review-agent | code-evaluator-agent | null",
  "current_skill": "harness-code-review | harness-code-evaluator | fix-self-check | null",
  "worktree_path": ".worktrees/plan-a",
  "current_task": "t-001",
  "tasks_completed": ["t-001"],
  "tasks_remaining": ["t-002", "t-003"],
  "attempt_counts": {
    "t-001": {"count": 1, "error_signature": null, "strategies_tried": []},
    "t-003": {"count": 3, "error_signature": "...", "strategies_tried": ["...", "..."]}
  },
  "fixing": null,
  "blocked_reason": null,

  "plan_status": {"plan-a": "merged", "plan-b": "running"}
}
```

> `plan_status` 仅主 state（`{module}-workflow-state.json`）含。子 plan state（`{module}-{plan}-state.json`）不含。单 Plan 场景主 state 的 `plan_status` 为空或不存在。

> **Plan 依赖关系**：存在独立的 `{module}-plan-flow.json`（与 state.json 同目录），由 AI 首次分析依赖后写入，格式 `{"plans": [{"name": "...", "path": "...", "depends_on": [...]}]}`。脚本 `workflow-todo-write.js` 据此拓扑排序算 Wave 分组。断点续传时直接复用，不需重新分析。

## stage 流转

```
coding → reviewing → reviewed → evaluating → evaluated → stage_completed → completed
             ↓            ↓
          fixing       fixing
             ↓            ↓
         reviewed     evaluated    ← 修复成功，回到上一个完成态
             ↓            ↓
          blocked      blocked      ← 超限
```

| stage | 含义 | 写入者 |
|-------|------|--------|
| `coding` | executor 编码中 | 主 Agent（派发 executor 前） |
| `reviewing` | 检视进行中 | 主 Agent（拉起 code-review-agent 前） |
| `reviewed` | 检视通过 | 主 Agent |
| `evaluating` | 评估进行中 | 主 Agent（拉起 code-evaluator-agent 前） |
| `evaluated` | 评估通过 | 主 Agent |
| `fixing` | 修复循环中 | 主 Agent（`fixing` 字段记录 trigger_stage + round） |
| `stage_completed` | 单个 Plan merge + worktree 清理完成 | 主 Agent |
| `completed` | 所有 Plan 全部完成，workflow 终态 | 主 Agent |
| `blocked` | 阻断 | executor 或主 Agent |

## 字段语义

| 字段 | 何时非空 | 用途 |
|------|---------|------|
| `truth_source` | 始终 | 需求来源（Plan 路径或 issue 报告路径），标识归属 |
| `stage` | 始终 | 精确定位恢复点（唯一状态字段） |
| `current_agent` | 主 Agent 直接加载的sub-agent，skill拉起的agent可以不记录 | 当前运行的 sub-agent 类型，恢复时重新派发该 agent |
| `current_skill` | 主 Agent 加载了 skill 时 | 当前加载的编排 skill 名称，恢复时需重新加载 |
| `worktree_path` | 多 Plan 模式 | 恢复时核对 worktree 一致性 |
| `current_task` | stage=coding/fixing | 续传下一个 task/issue |
| `tasks_completed` | stage=coding/fixing | 已完成项，不重复 |
| `tasks_remaining` | stage=coding/fixing | 待完成项 |
| `attempt_counts` | stage=coding/fixing | 每个 task/issue 的重试次数，恢复时知道"已试几次" |
| `fixing` | stage=fixing | `{"trigger_stage": "reviewing\|evaluating", "round": 1}` |
| `blocked_reason` | stage=blocked | 上报用户 |
| `plan_status` | 仅主 state，多 Plan 场景 | `{"plan_name": "running\|completed\|blocked\|merged"}`，记录各 plan 的编排级状态 |

## 状态机与刷新时机

**state.json 的 stage 字段在主 Agent 和 executor 各自的切换处刷新。**
主 Agent 负责编排阶段（reviewing / evaluating / fixing / completing）；executor 只负责执行阶段（coding / fixing）。

### Sub-agent（code-executor-agent）状态机

| stage | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | 主 Agent 派发 executor(mode=coding) | 全部 task 完成（build+test 均 pass） | `stage="coding"`, `status="running"`, `mode="coding"` | 每 task: `tasks_completed += [id]`, `tasks_remaining -= [id]`;全部完成后: `status="completed"` |
| `fixing` | 主 Agent 派发 executor(mode=fix) | 全部 issue 修复完成（build+test 均 pass） | `stage="fixing"`, `status="running"`, `mode="fix"` | 每 issue: `tasks_completed += [id]`, `tasks_remaining -= [id]`;全部完成后: `status="completed"` |

> executor 不进入 reviewing / evaluating 阶段。build+test 自检失败时按 deviation rule 在 coding/fixing 阶段内部修复（每 task/issue 最多 3 次），超过则 `status="blocked"`。

### Workflow（主 Agent）状态机

| stage | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | 单 Plan: 派发 executor(mode=coding);多 Plan: 串行或并发派发多个 executor(mode=coding) | 所有 executor 返回 status=completed | `stage="coding"`, `plan_status[plan]="running"` | 全部 executor `status="completed"` 后: `stage="reviewing"` |
| `reviewing` | coding 完成 | 通过 → `evaluating`；失败 → `fixing` | `stage="reviewing"` | 通过: `current_skill=null`；失败: `stage="fixing"` + `fixing={trigger_stage:"reviewing", reports}` + `attempt_counts++` |
| `evaluating` | reviewing 通过 | 通过 → `completing`；失败 → `fixing` | `stage="evaluating"`, `current_skill="evaluator"` | 通过: `current_skill=null`；失败: `stage="fixing"` + `fixing={trigger_stage:"evaluating", reports}` + `attempt_counts++` |
| `fixing` | reviewing 或 evaluating 失败 | 修完 → 回到 trigger_stage；超限 → blocked | `stage="fixing"`, `fixing={...}` | 修完: `stage={trigger_stage}`, `fixing=null`；超限: `status="blocked"` |
| `completing` | evaluating 通过 | `status="completed"` | `stage="completing"`, `current_plan=null`, `current_task=null`, `current_skill=null` | `status="completed"` |

### Stage 转换校验（per-plan，读 plan state 时隐式执行）

每个 stage 转换点在读 plan state 时隐式校验，不满足则该 plan 停在当前 stage，不推进。多 Plan 场景下不影响同 Wave 其他 plan。

| 转换点 | 校验项（读 plan state + 文件系统） |
|--------|----------------------------------|
| coding → reviewing | executor 返回 status=completed + 该 plan SUMMARY 已生成 |
| reviewing → evaluating | 检视报告已生成 + 无 HIGH blocking issues（或修复循环已达上限 BLOCKED） |
| evaluating → stage_completed | 评估报告已生成 + 无 HIGH blocking issues（或修复循环已达上限 BLOCKED） |
| Wave {N-1} → Wave {N}（多 Plan） | Wave {N-1} 所有 plan stage=stage_completed（有依赖模式: 已 merge） |

> 任一校验不满足 → 该 plan 停在当前 stage（修复循环内则继续循环或 BLOCKED）。校验不单独成 Gate 章节，融入 workflow 调度循环的转换点。

## 多 Plan 三层状态一致性

多 Plan 场景涉及三层状态：主 state（编排进度）+ 各 executor state（执行进度）+ worktree（代码隔离）。三者必须保持一致。

| 时机 | 主 state 写入 | executor state 写入 | worktree 操作 |
|------|-------------|---------------------|--------------|
| 创建 worktree + 派发 executor(mode=coding) | `plan_status[plan]="running"` | executor 初始化: `status="running"`, `mode="coding"`, `stage="coding"` | `git worktree add` |
| executor 每完成一个 task | — | `tasks_completed += [id]`, `tasks_remaining -= [id]`, `current_task=next` | — |
| executor 完成（主 Agent 读其 state 后） | `plan_status[plan]="completed"` 或 `"blocked"` | `status="completed"` 或 `"blocked"` | — |
| 派发 executor(mode=fix) 修复 | `fixing={trigger_stage, reports}` | executor: `status="running"`, `mode="fix"`, `stage="fixing"` | — |
| 全部 plan_status="completed" 后 | `stage="reviewing"` | — | — |
| Plan merge 到 main 后 | `plan_status[plan]="merged"` | — | — |
| worktree 清理后 | — | — | `git worktree remove` + `git branch -d` |

### 断点续传的一致性保证

续传时主 Agent 必须核对三层状态的一致性：

| 不一致情况 | 处理 |
|-----------|------|
| `plan_status[plan]="running"` 但 executor state 不存在 | executor 从未启动 → 正常派发 executor(mode=coding) |
| `plan_status[plan]="running"` 且 executor state `status="completed"` | executor 已完成但主 state 未更新 → 更新 `plan_status[plan]="completed"` |
| `plan_status[plan]="completed"` 但 worktree 不存在 | worktree 丢失 → 检查分支：存在则重建 worktree；不存在则该 Plan 需重新编码 |
| `plan_status[plan]="merged"` 但 worktree 仍存在 | worktree 未清理 → `git worktree remove` 清理 |

> **核心约束**：AI 不调 Write 工具，state.json 就不会更新。续传完全依赖 state 文件，上表每个时机都必须刷新。

## 状态恢复

### 恢复入口

扫描 `.opencode/harness/state/` 下所有 `*-state.json` 文件（每个文件对应一个 Plan）：

1. 不存在任何 state 文件 → 全新启动
2. 存在 state 文件 → 逐个读取，按 `stage` 恢复

### 逐 Plan 恢复

| stage | 恢复动作 |
|-------|---------|
| `coding` | 读 `current_agent` + `current_task` + `tasks_remaining` + `attempt_counts` → 重新派发该 agent 续传 |
| `reviewing` | 读 `current_skill`（若有）重新加载 → 重新拉起 `current_agent`（code-review-agent） |
| `reviewed` | 跳过（等待全部检视完成） |
| `evaluating` | 读 `current_skill`（若有）重新加载 → 重新拉起 `current_agent`（code-evaluator-agent） |
| `evaluated` | 跳过（等待全部评估完成） |
| `fixing` | 读 `current_skill`（若有）+ `fixing` 字段 → 从对应 trigger_stage 和 round 续传修复循环 |
| `stage_completed` | 跳过 |
| `completed` | 跳过（workflow 已完成） |
| `blocked` | `question()` 上报，附 `blocked_reason` |

### worktree 一致性检查

| 情况 | 处理 |
|------|------|
| `stage` 非 `stage_completed`/`blocked`，但 `worktree_path` 目录不存在 | worktree 丢失 → 检查 `workflow/{plan_name}` 分支：存在 → 重建；不存在 → 从头开始 |
| `stage` = `stage_completed`，但 worktree 仍存在 | 未清理 → `git worktree remove` |

### 全局恢复

所有 Plan 按 stage 恢复后：
- 有 `coding` 的 Plan → 继续 Phase 1
- 有 `reviewing`/`fixing` 的 Plan → 继续 Phase 2
- 有 `evaluating`/`fixing` 的 Plan → 继续 Phase 3
- 全部 `stage_completed` → 主 Agent 将所有 Plan state 刷新为 `completed`，workflow 收尾
