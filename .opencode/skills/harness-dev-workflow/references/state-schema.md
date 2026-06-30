# State 管理

**所有 state.json 使用统一 schema**,字段按角色(执行 vs 编排)填充 null/非 null。

## 统一 Schema

```json
{
  "module": "fdb-kvdb",
  "plan": "...plan-a.md | null",
  "truth_source_path": ["...plan-a.md"],
  "status": "running | blocked | completed",
  "last_run": "2026-06-30T10:30:00Z",
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

> `module` 字段为运行时参数，由 workflow 启动时从 `module_name` 输入或从 plan 文件名推断。上例以 `fdb-kvdb` 为示例。

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
