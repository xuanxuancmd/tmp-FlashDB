# 单 Plan 执行 Workflow

主 Agent 作为编排者，通过拉起 sub-agent 完成全闭环。单 Plan 模式**不需要 worktree**（无并发，executor 直接在项目根目录工作）。

> **stage 顺序由 `workflow.yaml` 的 local-stages / global-stages 数组定义**。本文件描述派发细节，不重复 stage 顺序。

## 完整流程

```
local-stages 顺序执行（code → review → evaluate，按 yaml 数组）
  → local 末项（evaluate）通过 → stage=stage_completed
  → [单 Plan 无 merge] → 直接进入 global-stages
  → global-stages 顺序执行（按 yaml 数组）
  → global 末项通过 → stage=completed
```

## 调度规则：照 todo 推进

> todo 唯一数据源 + 反模式见 SKILL.md "todo 使用约束"段。本段仅描述单 Plan 的刷新流程。

### 刷新流程

1. 写 state.json → hook 自动调脚本 → tool output 中出现 todos JSON
2. **立即**用该 JSON 调 `TodoWrite`（content / status / priority 逐字复制，不修改不补充）
3. 读 TodoWrite 结果，找 `status=in_progress` 项 → 执行对应 stage 派发（见"各 stage 派发细节"）
4. sub-agent 返回 → 刷新 state.json → 回到步骤 1
5. 出现 "Workflow 完成" 项且 `in_progress` → 执行 Self-Check

## 各 stage 派发细节

### local-stages: code（stage=coding）

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

- `status=completed` → 读 SUMMARY 确认编码完成 → 按 yaml local-stages 顺序刷新 `stage` = 下一项 → 进入下一 stage
- `status=blocked` → 汇总 blocked-reports → `question()` 上报用户

### local-stages: review（stage=reviewing）

加载检视编排 skill（{incremental_reviewing_skill}），按 skill 指引拉起 code-review-agent sub-agent（`review_path` = 模块 src 目录，增量检视），读 summary.json：
- `pass=true` → 按 yaml local-stages 顺序刷新 `stage` = 下一项 → 进入下一 stage
- `pass=false` → 提取 `blocking_issues[]` → 查 yaml 的 `on_failure` → 跳转 fix（引用 `fixing-loop.md`，trigger_stage=reviewing，max 3 轮）

### local-stages: evaluate（stage=evaluating）

直接派发 code-evaluator-agent sub-agent（无需加载编排 skill）：

```
task(
  subagent_type="code-evaluator-agent",
  description="代码评估",
  prompt="""
    requirement_source: Plan 路径: {plan_path}
    worktree_path:
  """
)
```

等待返回报告路径 → Read `.opencode/harness/evidence/code-evaluator-agent-review.json`：
- `overall_result.pass=true` → 刷新 `stage=stage_completed` → 进入 global-stages
- `pass=false` → 提取 `blocking_issues[]` → 查 yaml 的 `on_failure` → 跳转 fix（引用 `fixing-loop.md`，trigger_stage=evaluating，max_rounds 从 config.toml 读取）

### global-stages（stage=各 global-stages name）

local 末项通过 → `stage=stage_completed` → 直接进入 global-stages 第一项（单 Plan 无 merge）。

每个 global-stage 按 yaml 数组顺序执行，加载对应 skill，拉起 sub-agent：
- `pass=true` → 按 yaml global-stages 顺序刷新 `stage` = 下一项
- `pass=false` → 查 yaml 的 `on_failure` → 跳转 fix
- global 末项通过 → `stage=completed`

## Self-Check

在刷新 state `status=completed` 前，**回溯验证**：

1. state.json stage 曾经过 local-stages 全部 + global-stages 全部
2. 编码 SUMMARY、检视报告、评估报告、global-stages 报告均存在
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
- ✅ global-stages 全部通过: <各 stage 结果>
- ✅ 构建测试: `{build_cmd}` pass / `{test_cmd}` pass

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
