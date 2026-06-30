# State 管理

## Schema

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
  "blocked_reason": null
}
```

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

> executor 不改 `stage`，只在 `coding`/`fixing` 期间写 `current_task` 和 `tasks_*`。

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
