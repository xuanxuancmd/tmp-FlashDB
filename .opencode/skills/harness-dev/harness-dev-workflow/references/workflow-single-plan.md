# 单 Plan 执行 Workflow

主 Agent 作为编排者，通过拉起 sub-agent 完成全闭环。单 Plan 模式**不需要 worktree**（无并发，executor 直接在项目根目录工作）。

## 调度规则：照 todo 推进

主 Agent **照脚本输出的 todo 推进**，无需自行维护阶段清单：

1. 写初始主 state.json（`stage=coding`）→ hook 自动调脚本 → 输出 4 项 todo（编码 in_progress，其余 pending）
2. 调 `TodoWrite` 初始化 → 读 todo 找 `in_progress` 项 → 执行对应阶段派发（见下方"各阶段派发细节"）
3. sub-agent 返回 → 刷新 state.json → hook 自动输出新 todo → 调 `TodoWrite` 更新
4. 重复步骤 2-3，直到末尾 "Workflow 完成" 项变 `in_progress` → 执行 Self-Check

> stage 推进、修复轮次标注**全部由脚本**从 state.json 计算。AI 不参与 todo 状态计算，只按 todo 执行。

## 各阶段派发细节

### Phase 1: 编码

```
task(
  subagent_type="code-executor-agent",
  description="编码: {plan_name}",
  prompt="""
    mode: coding
    plan_path: {plan_path}
    module_name: {module_name}
    state_path: .opencode/harness/state/{module}-workflow-state.json
    worktree_path:
  """
)
```

- `status=completed` → 读 SUMMARY 确认编码完成 → 刷新 `stage=reviewing` → 进入 Phase 2
- `status=blocked` → 汇总 blocked-reports → `question()` 上报用户

### Phase 2: 检视（若无检视技能则跳过此阶段）

加载检视编排 skill：

| 技能 | 用途 |
|------|------|
| harness-code-review | Harness编码完成后的检视技能 |
| harness-run-e2e-test | E2E验证技能 |

按 skill 指引拉起 code-review-agent sub-agent（`review_path` = 模块 src 目录，全量检视），读 summary.json：
- `pass=true` → 刷新 `stage=evaluating` → 进入 Phase 3
- `pass=false` → 提取 `blocking_issues[]` → 进入检视修复循环（引用 `fixing-loop.md`，max 3 轮）

### Phase 3: 评估

加载评估编排 skill `harness-code-evaluator`，拉起 code-evaluator-agent sub-agent，读 JSON 报告：
- `pass=true` → 刷新 `stage=stage_completed` → 进入 Self-Check
- `pass=false` → 提取 `blocking_issues[]` → 进入评估修复循环（引用 `fixing-loop.md`，max 5 轮）

## Self-Check

在刷新 state `status=completed` 前，**回溯验证**：

1. state.json stage 曾经过 `coding → reviewing → evaluating`
2. 编码 SUMMARY、检视报告（或跳过已记录）、评估报告均存在
3. todo 所有项已 `completed`

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## 最终总结输出

```markdown
## 编码闭环完成

### 修改范围
- **Plan**: {plan_path}
- **模块**: {module_name}
- **涉及文件**: <从 git log 或 SUMMARY 提取>

### 结果
- ✅ 编码完成: <N> 个 task 全部实现
- ✅ 检视通过: <或"经 N 轮修复后通过">
- ✅ 评估通过: <或"经 N 轮修复后通过">
- ✅ 构建测试: `cargo check` pass / `cargo test` pass

### 结果举证
| 阶段 | 产物 | 路径 |
|------|------|------|
| 编码 | SUMMARY | {state_path 同级}/SUMMARY.md |
| 编码 | commit | <commit hash 列表> |
| 检视 | 报告 | .opencode/harness/evidence/{module}-review-report.md |
| 检视 | 摘要 | .opencode/harness/evidence/{module}-review-summary.json |
| 评估 | 报告 | .opencode/harness/evidence/code-evaluator-agent-review.md |
| 评估 | 摘要 | .opencode/harness/evidence/code-evaluator-agent-review.json |
| 状态 | state | {state_path} |
```
