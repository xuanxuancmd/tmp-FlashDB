# 多 Plan 执行 Workflow（worktree 隔离 + 串行/并发可选）

`plan_list.length > 1` 时，为每个 Plan 创建独立的 git worktree 并派发 `code-executor` sub-agent。

> **stage 顺序由 `workflow.yaml` 的 local-stages / global-stages 数组定义**。本文件描述派发细节和 merge 行为，不重复 stage 顺序。

## 完整流程

```
各 plan 各自走 local-stages（code → review → evaluate，按 yaml 数组）
  → 各自 stage=stage_completed
  → wait-for-all 屏障（同 Wave 所有 plan stage_completed）
  → 主 Agent 逐个 merge + 清理 worktree → plan_status=merged
  → 所有 plan merged → 进入 global-stages
  → global-stages 顺序执行（按 yaml 数组）
  → global 末项通过 → stage=completed
```

## 执行模式：有依赖 vs 无依赖

| 模式 | 适用条件 | merge 时机 | worktree |
|------|---------|-----------|---------|
| **有依赖模式**（默认） | 存在任意 plan 依赖另一 plan 的代码产物 | 每个 plan `stage=stage_completed` 时立即 merge（见 merge 行为专节） | 逐 plan 创建，后序从已 merge 的 main fork |
| **无依赖模式** | 所有 plan 互不依赖 | 集中在所有 plan stage_completed 后统一 merge | 可一次性创建，并行编码 |

> 有依赖模式下，后序 Wave 的 plan 从已 merge 的 main fork worktree，确保看到前序代码。

## 调度规则：照 todo 推进

> todo 唯一数据源 + 反模式见 SKILL.md "todo 使用约束"段。本段仅描述多 Plan 的刷新流程。

### 刷新流程

1. 写主 state.json（`plan_status` 全 `running`）→ hook 自动调脚本 → tool output 中出现 todos JSON（每 plan 1 项，Wave1 in_progress，其余 pending）
2. **立即**用该 JSON 调 `TodoWrite`（content / status / priority 逐字复制，不修改不补充）
3. 读 TodoWrite 结果，找 `status=in_progress` 项 → 执行对应 stage 派发（见下方"各 stage 派发细节"）
4. sub-agent 返回 → 刷新 plan state.json → 回到步骤 1（脚本输出新 todo）
5. 所有 plan merge 完成 + todo 出现 global-stages 项 → 执行 global-stages + Self-Check

> Wave 分组、依赖状态、stage 推进**全部由脚本**从 state.json + plan-flow.json + workflow.yaml 计算。todo 中每 plan 只占 1 项（当前活跃 stage 的动作），不做全景投影。

## Step 0: 初始化（强制执行，不可跳过）

### 0.0 断点续传检查（若主 state 已存在）

若主 state.json 已存在且 `status=running`，按 `references/state-schema.md` 的"状态恢复"段执行（含三层状态一致性核对 + worktree 重建）。全新启动跳过本步。

### 0.1 依赖分析 + 写 plan-flow.json

读所有 plan，分析依赖关系，写入 `harness/state/{module}-plan-flow.json`：

```json
{
  "plans": [
    {"name": "{plan_name}", "path": "{plan_path}", "depends_on": ["{前置plan}", ...]}
  ]
}
```

**依赖判断**：若 plan 文件 frontmatter 含 `depends_on` 字段 → 直接使用；否则 → 主 Agent 根据各 plan 的修改文件范围和目标判断。

> plan-flow.json 是流程级文件，首次写入后断点续传时直接复用，不需重新分析。

### 0.2 创建 worktree

```bash
git worktree add .worktrees/{plan_name} -b workflow/{plan_name}
```

- worktree 路径在项目目录内（`.worktrees/`），sub-agent 无需 `external_directory` 权限
- **确保 `.worktrees/` 已加入 `.gitignore`**
- **有依赖模式**：逐 plan 创建，该 plan 闭环完成立即 merge + 删除 worktree，再为后序 plan 从已 merge 的 main 重新 fork
- **无依赖模式**：可一次性创建所有 worktree（并行编码），所有 plan stage_completed 后统一 merge 后清理

## 各 stage 派发细节

以下为每个 plan 的 per-plan 闭环各阶段的派发方式。各阶段**不是全局屏障**——某 plan 走完编码即进检视，不等同 Wave 其他 plan。

### local-stages: code（stage=coding）

```
task(subagent_type="code-executor-agent",
     description="编码: {plan_name}",
     prompt="""
       mode: coding
       plan_path: {plan_path}
       module_name: {module}
       state_path: .opencode/harness/state/{module}-{plan}-state.json
       worktree_path: .worktrees/{plan_name}
     """)
```

- 同 Wave 内多个 plan 可加 `background=true` 并行派发
- executor 返回后读该 plan state → `status=completed` → 刷新主 state `plan_status[plan]="completed"` + 该 plan state `stage` = local-stages 下一项
- `status=blocked` → 该 plan 标记 BLOCKED，不影响同 Wave 其他 plan

### local-stages: review（stage=reviewing）

```
task(subagent_type="code-review-agent",
     description="检视: {plan_name}",
     prompt="检视路径: .worktrees/{plan_name}/src/...")
```

读 summary.json：
- `pass=true` → 按 yaml local-stages 顺序刷新该 plan state `stage` = 下一项
- `pass=false` → 提取 `blocking_issues[]` → 查 yaml 的 `on_failure` → 跳转 fix（引用 `fixing-loop.md`，trigger_stage=reviewing，max 3 轮）

### local-stages: evaluate（stage=evaluating）

直接派发 code-evaluator-agent sub-agent（无需加载编排 skill）：

```
task(subagent_type="code-evaluator-agent",
     description="评估: {plan_name}",
     prompt="""
       requirement_source: Plan 路径: {plan_path}
       worktree_path: .worktrees/{plan_name}
     """)
```

等待返回报告路径 → Read JSON 报告：
- `pass=true` → 刷新该 plan state `stage=stage_completed`
- `pass=false` → 提取 `blocking_issues[]` → 查 yaml 的 `on_failure` → 跳转 fix（引用 `fixing-loop.md`，trigger_stage=evaluating，max_rounds 从 config.toml 读取）

### 修复循环（per-plan，引用 fixing-loop.md）

检视/评估未通过的 plan 独立进入修复循环，逻辑见 `fixing-loop.md`。同一 plan 的修复→重检视/重评估是串行的；不同 plan 的修复循环可并发。

## merge 行为（主 Agent 内置编排动作）

> merge **不在 `workflow.yaml` 中定义**——merge 是主 Agent 检测所有 plan `stage=stage_completed` 后的内置编排动作（依赖多 plan 状态，属 wait-for-all 屏障）。

### wait-for-all 屏障

主 Agent 检测**同 Wave 所有非 blocked plan** `stage=stage_completed` 后，才执行 merge。任一 plan 未到 stage_completed → 等待。

### merge 命令

满足屏障条件后，主 Agent 逐个执行：

```bash
git merge workflow/{plan_name}          # merge 到 main
git worktree remove .worktrees/{plan_name}  # 清理 worktree
git branch -d workflow/{plan_name}      # 删除分支
```

- merge 完成 → 主 state `plan_status[plan]="merged"`
- 所有 plan merged 后 → 进入 global-stages
- **有依赖模式**：各 plan 的 merge 已在其 stage_completed 时立即执行（不等同 Wave），后序 Wave 的 plan 从已 merge 的 main fork worktree
- **无依赖模式**：所有 plan stage_completed 后集中 merge
- merge 冲突 → 主 Agent 直接解决；冲突复杂无法自动解决 → `question()` 上报用户

### BLOCKED Plan 的处理

- ❌ 不 merge（代码未通过检视/评估）
- ❌ 不删除 worktree（保留现场供调试）
- ✅ `question()` 上报用户，附 blocked-reports 路径

## global-stages（所有 plan merge 完成后执行）

所有 plan `plan_status=merged` 后，进入 global-stages。每个 global-stage 按 yaml 数组顺序执行：

- 加载对应 skill，拉起 sub-agent
- `pass=true` → 按 yaml global-stages 顺序刷新主 state `stage` = 下一项
- `pass=false` → 查 yaml 的 `on_failure` → 跳转 fix
- global 末项通过 → `stage=completed`

> todo 中 plan 项消失，出现 global 项（`Global: {stage_name} — 派 {skill}`）。

## Self-Check

在刷新主 state `status=completed` 前，**回溯验证**：

1. 所有 `plan_status="merged"`（blocked 的除外）
2. 每个 Plan 的编码 SUMMARY、检视报告、评估报告均存在
3. 所有 Plan 已 merge 到 main（git log 确认）
4. 所有 worktree 已清理（blocked 的保留现场除外）
5. global-stages 全部通过
6. todo 末尾 "Workflow 完成" 项已 `completed`

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## 最终总结输出

```markdown
## 编码闭环完成

### 修改范围
- **Plan 列表**: <列出所有 plan_path>
- **模块**: {module_name}
- **执行模式**: 有依赖 / 无依赖
- **涉及文件**: <从 git log 或各 Plan SUMMARY 提取>

### 结果
| Plan | Wave | 编码 | 检视 | 评估 | merge |
|------|------|------|------|------|-------|
| {plan_name} | {N} | ✅ N task | ✅ / N 轮修复 | ✅ / N 轮修复 | ✅ commit abc123 |

- ✅ 所有 Plan 已 merge 到 main
- ✅ 所有 worktree 已清理
- ✅ global-stages 全部通过
- ✅ 最终构建测试: `{build_cmd}` pass / `{test_cmd}` pass

### 结果举证
| 阶段 | 产物 | 路径 |
|------|------|------|
| 编码 | SUMMARY | .opencode/harness/state/{module}-{plan}-SUMMARY.md（每个 Plan） |
| 检视 | 报告 | .opencode/harness/evidence/{module}-review-report.md（每个 Plan） |
| 评估 | 报告 | .opencode/harness/evidence/code-evaluator-agent-review.md（每个 Plan） |
| 状态 | state | .opencode/harness/state/{module}-workflow-state.json |
```
