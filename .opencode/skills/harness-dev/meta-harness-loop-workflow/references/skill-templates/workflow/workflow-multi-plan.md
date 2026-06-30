# 多 Plan 执行 Workflow（worktree 隔离 + 串行/并发可选）

`plan_list.length > 1` 时，为每个 Plan 创建独立的 git worktree 并派发 `code-executor` sub-agent（`mode: subagent`，全新上下文）。

## 执行模式：串行 vs 并发

**默认串行执行所有 Plan**。是否并发取决于上下文：

| 模式 | 适用条件 | worktree | task 调用 |
|------|---------|---------|---------|
| **串行**（默认） | Plan 间有文件依赖、项目规模不大、无需加速 | 每个 Plan 独立 worktree，前一个 merge 后复用或新建 | `background=false`（逐个等待） |
| **并发**（可选） | Plan 间无文件依赖、需加速 | 每个 Plan 独立 worktree | `background=true`（并行派发） |

## 架构

主 Agent 是**纯编排者**，协调三类 sub-agent:
- executor(mode=coding / mode=fix):每个 Plan 独立编码/修复
- code-review-agent:每个 Plan 独立检视
- code-evaluator-agent:每个 Plan 独立评估

**worktree 隔离**:每个 Plan 一个独立 worktree（位于项目根目录 `.worktrees/{plan_name}/`），所有 sub-agent 通过 `worktree_path` 参数在该 worktree 中工作。Plan 文件本身不在 worktree 内——它是 workflow 的输入，从其原始路径读取。

## 总流程

```
[主 Agent Step 0 编排清单 + worktree 创建]
  → Phase 1: 多plan编码 — 每个 Plan 派 executor(mode=coding)（串行或并发）
  → Phase 2: 多plan检视 — 每个 Plan 派 code-review-agent（串行或并发）+ 修复循环
  → Phase 3: 多plan评估 — 每个 Plan 派 code-evaluator-agent（串行或并发）+ 修复循环
  → Phase 4: 逐个 merge + worktree 清理
  → [主 Agent completing + Self-Check]
```

## 主 Agent Step 0: 编排清单初始化 + worktree 创建（强制执行，不可跳过）

主 Agent 在启动任何 sub-agent 前，**必须**：

### 0.0 断点续传检查（若主 state 已存在）

若主 state.json 已存在且 status="running"，按 `references/state-schema.md` 的"多 Plan 恢复"段执行：

1. **Read 主 state** → 确定恢复入口（phase）
2. **逐 Plan 恢复** → 检查 `plan_status`，对 `running` 的 Plan Read 其 executor state
3. **worktree 一致性检查** → 核对 `.worktrees/` 与 `plan_status`，缺失的 worktree 重建（用已有分支 `git worktree add .worktrees/{plan_name} workflow/{plan_name}`），多余的 worktree 清理
4. **按恢复结果跳转到对应 Phase** → 无需重新执行 Step 0.1-0.3

> 全新启动（无 state 文件）时跳过本步，继续 0.1。

### 0.1 文件依赖分析 + 执行模式决策

由主根据上下文判断：

- 有依赖的 Plan **必须串行**
- 无依赖冲突的 Plan **可并发**

### 0.2 创建 worktree

为每个 Plan 创建独立 worktree（无论串行还是并发，都需 worktree 隔离避免 git index.lock 冲突）。worktree 创建在**项目根目录**下，不在 harness 目录内（harness 只放 state/logs/scripts/evidence）：

```bash
git worktree add .worktrees/{plan_name} -b workflow/{plan_name}
```

> - worktree 路径在项目目录内（`.worktrees/`），sub-agent 无需 `external_directory` 权限
> - 分支名用 `workflow/{plan_name}` 前缀，避免与 Plan 文档本身混淆
> - **确保 `.worktrees/` 已加入 `.gitignore`**（worktree 是临时工作目录，不应提交）
> - Plan 文件从其原始路径读取（如 `.omo/plans/{plan_name}.md`），不在 worktree 内
> - **串行模式下**：可逐个创建 worktree（前一个 Plan merge 后删除 worktree，再创建下一个），也可一次性全部创建

### 0.3 TodoWrite 编排清单

```
1. ☐ Phase 1: 派发每个 Plan 的 executor(mode=coding)（串行或并发）
2. ☐ Phase 1: 等待所有 executor 完成（读各 plan state）
3. ☐ Phase 2: 派发 code-review-agent（串行或并发）+ 修复循环（max 3 轮/plan）
4. ☐ Phase 3: 派发 code-evaluator-agent（串行或并发）+ 修复循环（max 5 轮/plan）
5. ☐ Phase 4: 逐个 merge + worktree 清理
6. ☐ completing: 刷新主 state status=completed + Self-Check
```

## Phase 1: 多plan编码

主 Agent 为每个 Plan 派发独立的 `code-executor` sub-agent（mode=coding），传入 `worktree_path`。

**串行模式**（默认）：逐个派发，等待返回后再派下一个：

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

**并发模式**：加 `background=true` 并行派发，等待全部完成后统一汇总。

### Phase 1 等待 + 状态汇总

**串行模式**：每个 executor 返回后立即读 state，确认 status 后派下一个 Plan。
**并发模式**：所有 `background=true` 派发的 sub-agent 完成后，主 Agent 等待原生 `<task id="..." state="completed">` XML 通知，逐个读 plan state 文件。

两种模式均需：
- Read 各 plan 的 `state_path` → 确认 `status="completed"`
- 任一 plan status="blocked" → 主 Agent 汇总 blocked-reports，进入全局 fixing 或 `question()` 上报

## Phase 2: 多plan检视

### 2.1 触发检视

为每个 Plan 拉起独立的 code-review-agent sub-agent，`review_path` 指向 worktree 内的文件：

**串行模式**（默认）：逐个派发，等待返回。
**并发模式**（可选）：加 `background=true` 并行派发。

```
task(subagent_type="code-review-agent",
     description="检视: {plan_name}",
     prompt="检视路径: .worktrees/{plan_name}/src/...")
```

### 2.2 消费检视报告

逐个读各 Plan 的 summary.json:
- `pass=true` 的 Plan → 标记检视通过
- `pass=false` 的 Plan → 提取 `blocking_issues[]`，进入 2.3

### 2.3 检视修复循环（per-plan, max 3 轮）

对每个检视未通过的 Plan，独立进入修复循环:

```
round = 0
循环:
  round >= 3 → 该 Plan 标记 BLOCKED
  round += 1
  task(executor, mode=fix, plan={plan}, issues=blocking_issues,
       context_summary=SUMMARY, worktree_path=.worktrees/{plan_name})
  → 等待返回
  task(code-review-agent, review_path=.worktrees/{plan_name}/src/...)
  → 读 summary.json
  pass=true → 该 Plan 检视通过
  pass=false → 提取 blocking_issues → 返回循环
```

**并发模式下**：多个 Plan 的修复循环可并发执行（各 Plan 独立 background sub-agent），但同一 Plan 的修复→重检视是串行的。串行模式下逐个 Plan 处理。

### 2.4 检视阶段完成

所有 Plan 检视通过（或达到上限 BLOCKED）→ 进入 Phase 3。

## Phase 3: 多plan评估

### 3.1 触发评估

为每个 Plan 拉起独立的 code-evaluator-agent sub-agent：

**串行模式**（默认）：逐个派发，等待返回。
**并发模式**（可选）：加 `background=true` 并行派发。

```
task(subagent_type="code-evaluator-agent",
     description="评估: {plan_name}",
     prompt="""
       requirement_source: Plan 路径: {plan_path}
       worktree_path: .worktrees/{plan_name}
     """)
```

### 3.2 消费评估报告

逐个读各 Plan 的评估 JSON 报告:
- `pass=true` 的 Plan → 标记评估通过
- `pass=false` 的 Plan → 提取 `blocking_issues[]`，进入 3.3

### 3.3 评估修复循环（per-plan, max 5 轮）

逻辑同 2.3，但:
- 修复循环上限为 5 轮
- 每轮修复后重新拉起 code-evaluator-agent 评估
- executor(mode=fix) 的 `worktree_path` 指向该 Plan 的 worktree

### 3.4 评估阶段完成

所有 Plan 评估通过（或达到上限 BLOCKED）→ 进入 Phase 4。

## Phase 4: 逐个 merge + worktree 清理

对每个全闭环通过的 Plan（串行处理，避免 merge 冲突叠加）：

### 4.1 merge 到 main

```bash
git merge workflow/{plan_name}
```

- merge 冲突 → 主 Agent 直接解决
- 冲突复杂无法自动解决 → `question()` 上报用户

### 4.2 清理 worktree

```bash
git worktree remove .worktrees/{plan_name}
git branch -d workflow/{plan_name}
```

### 4.3 BLOCKED Plan 的 worktree 处理

- ❌ 不 merge（代码未通过检视/评估）
- ❌ 不删除 worktree（保留现场供调试）
- ✅ `question()` 上报用户，附 blocked-reports 路径

## completing Self-Check

在刷新主 state status=completed 前，**回溯验证**所有阶段已实际执行：

1. Read 主 state.json → 确认所有 plan_status="completed"
2. 确认每个 Plan 的编码 SUMMARY 存在
3. 确认每个 Plan 的检视报告存在（或 {review_skills} 为空已记录跳过）
4. 确认每个 Plan 的评估报告存在
5. 确认所有 Plan 已 merge 到 main（git log 确认）
6. 确认所有 worktree 已清理（git worktree list 确认）
7. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## 主 Agent HARD GATE — 全局阶段切换验证

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| Phase 1 → Phase 2 | ① 所有 plan status="completed" ② 各 plan SUMMARY 已生成 ③ 主 state phase="reviewing" ④ TodoWrite "Phase 1" 已 completed | 任一未满足 → BLOCKED |
| Phase 2 → Phase 3 | ① 各 plan 检视报告已生成（或跳过已记录）② 各 plan 无 HIGH blocking issues（或修复循环已达上限 BLOCKED）③ TodoWrite "Phase 2" 已 completed | 任一未满足 → BLOCKED，返回 Phase 2 |
| Phase 3 → Phase 4 | ① 各 plan 评估报告已生成 ② 各 plan 无 HIGH blocking issues（或修复循环已达上限 BLOCKED）③ TodoWrite "Phase 3" 已 completed | 任一未满足 → BLOCKED，返回 Phase 3 |
| Phase 4 → completing | ① 所有 plan 已 merge 到 main ② 所有 worktree 已清理 | 任一未满足 → BLOCKED，返回 Phase 4 |

## 主 Agent success_criteria

- [ ] 所有 Plan 的 executor(mode=coding) 已完成（plan_status 全部 completed）
- [ ] 所有 plan state.json 均已读回，status="completed"
- [ ] Phase 2 检视阶段已执行（或 {review_skills} 为空已跳过）
- [ ] Phase 3 评估阶段已执行（harness-code-evaluator 已调用）
- [ ] 所有 Plan 已 merge 到 main
- [ ] 所有 worktree 已清理
- [ ] 主 state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（所有阶段报告存在）

## 最终总结输出

workflow 完成后，主 Agent **必须**向用户输出以下总结（读取各 Plan 的 SUMMARY 和 evidence 报告汇总）：

```markdown
## 编码闭环完成

### 修改范围
- **Plan 列表**: <列出所有 plan_path>
- **模块**: {module_name}
- **执行模式**: 串行 / 并发
- **涉及文件**: <从 git log 或各 Plan SUMMARY 提取，列出新增/修改的文件清单>

### 目的
<一句话描述本次多个 Plan 的整体目标>

### 结果
| Plan | 编码 | 检视 | 评估 | merge |
|------|------|------|------|-------|
| plan-a | ✅ N task | ✅ / N 轮修复 | ✅ / N 轮修复 | ✅ commit abc123 |
| plan-b | ✅ N task | ✅ | ✅ | ✅ commit def456 |

- ✅ 所有 Plan 已 merge 到 main
- ✅ 所有 worktree 已清理
- ✅ 最终构建测试: `cargo check` pass / `cargo test` pass

### 结果举证
| 阶段 | 产物 | 路径 |
|------|------|------|
| 编码 | SUMMARY | .opencode/harness/state/{module}-{plan}-SUMMARY.md（每个 Plan） |
| 编码 | commit | <各 Plan 的 commit hash 列表> |
| 检视 | 报告 | .opencode/harness/evidence/{module}-review-report.md（每个 Plan） |
| 检视 | 摘要 | .opencode/harness/evidence/{module}-review-summary.json（每个 Plan） |
| 评估 | 报告 | .opencode/harness/evidence/code-evaluator-agent-review.md（每个 Plan） |
| 评估 | 摘要 | .opencode/harness/evidence/code-evaluator-agent-review.json（每个 Plan） |
| 状态 | state | .opencode/harness/state/{module}-workflow-state.json |
```
