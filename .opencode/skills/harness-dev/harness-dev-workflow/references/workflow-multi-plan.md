# 多 Plan 执行 Workflow（worktree 隔离 + 串行/并发可选）

`plan_list.length > 1` 时，为每个 Plan 创建独立的 git worktree 并派发 `code-executor` sub-agent。

## 执行模式：有依赖 vs 无依赖

| 模式 | 适用条件 | merge 时机 | worktree |
|------|---------|-----------|---------|
| **有依赖模式**（默认） | 存在任意 plan 依赖另一 plan 的代码产物 | 每个 plan `stage=stage_completed` 时立即 merge | 逐 plan 创建，后序从已 merge 的 main fork |
| **无依赖模式** | 所有 plan 互不依赖 | 集中在 Phase 4 统一 merge | 可一次性创建，并行编码 |

> 有依赖模式下，后序 Wave 的 plan 从已 merge 的 main fork worktree，确保看到前序代码。

## 调度规则：照 todo 推进

主 Agent **照脚本输出的 todo 推进**，无需自行计算 Wave 或调度顺序：

1. 写初始主 state.json（`stage=coding`, `plan_status` 全 `running`）→ hook 自动调脚本 → 输出全 `pending` todo（首次全景）
2. 调 `TodoWrite` 初始化 → 读 todo 找 `in_progress` 项 → 执行对应阶段派发（见下方"各阶段派发细节"）
3. sub-agent 返回 → 刷新 plan state.json → hook 自动输出新 todo → 调 `TodoWrite` 更新
4. 重复步骤 2-3，直到所有 plan merge 完成 + 末尾 "Workflow 完成" 项变 `in_progress` → 执行 Phase 4 + Self-Check

> Wave 分组、依赖状态、stage 推进**全部由脚本**从 state.json + plan-flow.json 计算。AI 不参与调度计算，只按 todo 执行。

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
- **无依赖模式**：可一次性创建所有 worktree（并行编码），Phase 4 统一 merge 后清理

## 各阶段派发细节

以下为每个 plan 的 per-plan 闭环各阶段（编码/检视/评估）的派发方式。各阶段**不是全局屏障**——某 plan 走完编码即进检视，不等同 Wave 其他 plan。

### 编码阶段（stage=coding）

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
- executor 返回后读该 plan state → `status=completed` → 刷新主 state `plan_status[plan]="completed"` + 该 plan state `stage=reviewing`
- `status=blocked` → 该 plan 标记 BLOCKED，不影响同 Wave 其他 plan

### 检视阶段（stage=reviewing）

加载检视编排 skill：

| 技能 | 用途 |
|------|------|
| harness-code-review | Harness编码完成后的检视技能 |
| harness-run-e2e-test | E2E验证技能 |

按 skill 指引拉起 code-review-agent sub-agent（`review_path` 指向 worktree 内文件）：

```
task(subagent_type="code-review-agent",
     description="检视: {plan_name}",
     prompt="检视路径: .worktrees/{plan_name}/src/...")
```

读 summary.json：
- `pass=true` → 刷新该 plan state `stage=evaluating`
- `pass=false` → 提取 `blocking_issues[]` → 进入检视修复循环（引用 `fixing-loop.md`，max 3 轮）

### 评估阶段（stage=evaluating）

```
task(subagent_type="code-evaluator-agent",
     description="评估: {plan_name}",
     prompt="""
       requirement_source: Plan 路径: {plan_path}
       worktree_path: .worktrees/{plan_name}
     """)
```

读 JSON 报告：
- `pass=true` → 刷新该 plan state `stage=stage_completed`
- `pass=false` → 提取 `blocking_issues[]` → 进入评估修复循环（引用 `fixing-loop.md`，max 5 轮）

### 修复循环（per-plan，引用 fixing-loop.md）

检视/评估未通过的 plan 独立进入修复循环，逻辑见 `fixing-loop.md`。同一 plan 的修复→重检视/重评估是串行的；不同 plan 的修复循环可并发。

## Phase 4: merge + worktree 清理

> - **有依赖模式**：各 plan 的 merge 已在其闭环 `stage=stage_completed` 时立即执行，Phase 4 仅做最终校验 + worktree 残留清理
> - **无依赖模式**：所有 plan `stage=stage_completed` 后，集中在本 Phase 逐个 merge

### 4.1 merge 到 main（仅无依赖模式执行）

```bash
git merge workflow/{plan_name}
```

- merge 冲突 → 主 Agent 直接解决；冲突复杂无法自动解决 → `question()` 上报用户

### 4.2 清理 worktree（两种模式均执行）

```bash
git worktree remove .worktrees/{plan_name}
git branch -d workflow/{plan_name}
```

### 4.3 BLOCKED Plan 的 worktree 处理

- ❌ 不 merge（代码未通过检视/评估）
- ❌ 不删除 worktree（保留现场供调试）
- ✅ `question()` 上报用户，附 blocked-reports 路径

## completing Self-Check

在刷新主 state `status=completed` 前，**回溯验证**：

1. 所有 `plan_status="merged"`（blocked 的除外）
2. 每个 Plan 的编码 SUMMARY、检视报告、评估报告均存在
3. 所有 Plan 已 merge 到 main（git log 确认）
4. 所有 worktree 已清理（blocked 的保留现场除外）
5. todo 末尾 "Workflow 完成" 项已 `completed`

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
- ✅ 最终构建测试: `cargo check` pass / `cargo test` pass

### 结果举证
| 阶段 | 产物 | 路径 |
|------|------|------|
| 编码 | SUMMARY | .opencode/harness/state/{module}-{plan}-SUMMARY.md（每个 Plan） |
| 检视 | 报告 | .opencode/harness/evidence/{module}-review-report.md（每个 Plan） |
| 评估 | 报告 | .opencode/harness/evidence/code-evaluator-agent-review.md（每个 Plan） |
| 状态 | state | .opencode/harness/state/{module}-workflow-state.json |
```
