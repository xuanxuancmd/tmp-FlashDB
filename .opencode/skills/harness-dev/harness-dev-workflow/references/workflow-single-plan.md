# 单 Plan 执行 Workflow

主 Agent 作为编排者,通过拉起 sub-agent 完成全闭环。单 Plan 模式**不需要 worktree**(无并发,executor 直接在项目根目录工作)。

## 流程

```
[Step 0 清单初始化]
  → [Phase 1: 编码] task(executor, mode=coding)
  → [Phase 2: 检视] task(code-review-agent) → [修复循环: task(executor, mode=fix) → 重检视] (max 3 轮)
  → [Phase 3: 评估] task(code-evaluator-agent) → [修复循环: task(executor, mode=fix) → 重评估] (max 5 轮)
  → [Phase 4: completing]
```

## Step 0: 流程清单初始化（强制执行，不可跳过）

在开始任何工作前，**必须**调用 TodoWrite 创建完整流程清单。跳过此步骤视为流程违规。

### todo 模板

```
1. ☐ Phase 1: 编码 — task(executor, mode=coding)
2. ☐ Phase 2: 检视 — task(code-review-agent) + 修复循环（max 3 轮）
3. ☐ Phase 3: 评估 — task(code-evaluator-agent) + 修复循环（max 5 轮）
4. ☐ Phase 4: 完成 — 刷新 state status=completed + Self-Check
```

> TodoWrite 让阶段跳过变得"可见"。后续每个阶段完成时必须将对应项标记为 completed，completing 阶段会回溯校验。

## Phase 1: 编码

主 Agent 派发 executor sub-agent 执行编码。单 Plan 模式 `worktree_path` 为空(项目根目录)。

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

**等待返回** → 读返回信号:
- `status=completed` → 读 SUMMARY 确认编码完成 → 进入 Phase 2
- `status=blocked` → 汇总 blocked-reports → `question()` 上报用户

## Phase 2: 检视（若无检视技能则跳过此阶段）

### 2.1 触发检视

加载检视编排 skill:

| 技能 | 用途 |
|------|------|
| harness-code-review | 编码完成后的检视编排，驱动检视-修复循环（max 3 轮） |
| harness-run-e2e-test | 声明式 YAML 驱动的 E2E 端到端验证，生成结构化 evidence |

按 skill 指引拉起 code-review-agent sub-agent:
- `review_path` = 模块 src 目录（全量检视）
- sub-agent 返回报告路径

### 2.2 消费检视报告

按 skill 指引读 summary.json:
- `pass=true` → 进入 Phase 3
- `pass=false` → 提取 `blocking_issues[]` → 进入 2.3 修复循环

### 2.3 检视修复循环（max 3 轮）

```
round = 0
循环:
  round >= 3 → BLOCKED, question() 上报
  round += 1
  task(executor, mode=fix, issues=blocking_issues, context_summary=SUMMARY路径, worktree_path=)
  → 等待返回
  task(code-review-agent, review_path=变更文件)
  → 读 summary.json
  pass=true → 进入 Phase 3
  pass=false → 提取 blocking_issues → 返回循环
```

**修复时主 Agent 行为**:
1. 从 summary.json 提取 `blocking_issues[]`（JSON 摘要）
2. 拉起 executor(mode=fix)，传入 issues + 上一轮 SUMMARY 路径
3. executor 返回后，重新拉起 code-review-agent 检视
4. 重复直到 pass 或达到 3 轮上限

## Phase 3: 评估

### 3.1 触发评估

加载评估编排 skill:

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。

### 3.2 消费评估报告

按 skill 指引读 JSON 报告:
- `pass=true` → 进入 Phase 4
- `pass=false` → 提取 `blocking_issues[]` → 进入 3.3 修复循环

### 3.3 评估修复循环（max 5 轮）

逻辑同 2.3，但:
- 修复循环上限为 5 轮（evaluator skill 定义）
- 每轮修复后重新拉起 code-evaluator-agent 评估

## Phase 4: completing

### Self-Check

在刷新 state status=completed 前，**回溯验证**所有阶段已实际执行：

1. Read state.json → 确认 phase 曾经过 coding → reviewing → evaluating
2. 确认编码 SUMMARY 存在
3. 确认检视报告存在（或无检视技能已记录跳过）
4. 确认评估报告存在
5. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## HARD GATE — 阶段切换验证

每个阶段切换处必须通过以下验证，未通过则 BLOCKED，不得进入下一阶段。**这不是建议，未通过门禁的阶段切换是非法的。**

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| Phase 1 → Phase 2 | ① executor 返回 status=completed ② 编码 SUMMARY 已生成 ③ state.json phase="reviewing" ④ TodoWrite "Phase 1" 已 completed | 任一未满足 → BLOCKED，返回 Phase 1 |
| Phase 2 → Phase 3 | ① 检视报告已生成（或跳过已记录）② 报告无 HIGH blocking issues（或修复循环已达上限 BLOCKED）③ TodoWrite "Phase 2" 已 completed | 任一未满足 → BLOCKED，返回 Phase 2 |
| Phase 3 → Phase 4 | ① 评估报告已生成 ② 报告无 HIGH blocking issues（或修复循环已达上限 BLOCKED）③ TodoWrite "Phase 3" 已 completed | 任一未满足 → BLOCKED，返回 Phase 3 |

## success_criteria

- [ ] Phase 1 编码已执行（executor mode=coding 已调用，SUMMARY 已生成）
- [ ] Phase 2 检视已执行（或无检视技能已跳过）
- [ ] Phase 3 评估已执行（harness-code-evaluator 已调用）
- [ ] state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（所有阶段报告存在）

## 最终总结输出

workflow 完成后，主 Agent **必须**向用户输出以下总结（读取各阶段的 SUMMARY 和 evidence 报告汇总）：

```markdown
## 编码闭环完成

### 修改范围
- **Plan**: {plan_path}
- **模块**: {module_name}
- **涉及文件**: <从 git log 或 SUMMARY 提取，列出新增/修改的文件清单>

### 目的
<一句话描述本次 Plan 的目标，从 Plan 文件提取>

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
