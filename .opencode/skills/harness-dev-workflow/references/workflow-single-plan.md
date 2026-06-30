# 单 Plan 执行 Workflow

## 流程

```
[Step 0 清单初始化] → [编码 task 1..N] → [evaluating] → [reviewing] → [completing]
```
*此处`reviewing`等同于full_reviewing*

## Step 0: 流程清单初始化（强制执行，不可跳过）

在开始任何编码工作前，**必须**调用 TodoWrite 创建完整流程清单。跳过此步骤视为流程违规。

### todo 模板

```
1. ☐ 编码：实现 Plan 中所有 task
2. ☐ 对抗性评估（evaluating）：调用 harness-code-evaluator
3. ☐ 代码检视（reviewing）：加载检视技能执行（占位符为空则跳过）
4. ☐ 完成（completing）：刷新 state status=completed + Self-Check
```

> TodoWrite 让阶段跳过变得"可见"。后续每个阶段完成时必须将对应项标记为 completed，completing 阶段会回溯校验。

## 阶段详情

- **编码**:加载以下技能:

  | 技能 | 用途 |
  |------|------|
  | c-translate-to-rust | C→Rust 1:1 翻译规则与决策框架（强制翻译表、禁令表、易错表、复杂场景参考） |

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **reviewing**(占位符为空则跳过此阶段): 加载以下技能:

  | 技能 / Agent | 用途 |
  |--------------|------|
  | code-review-agent | 通用代码检视（全量模式，传入模块 src/ 目录路径触发） |
  | parity-verifier | 跨语言行为等价性验证（对照 C 源码与 Rust 译文检查行为一致性） |

- evaluating和reviewing 任一阶段失败 → Fixing(见 `fixing-loop.md`)。

## HARD GATE — 阶段切换验证

每个阶段切换处必须通过以下验证，未通过则 BLOCKED，不得进入下一阶段。**这不是建议，未通过门禁的阶段切换是非法的。**

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| 编码 → evaluating | ① Plan 中所有 task 已实现 ② state.json 已刷新 phase="evaluating" ③ TodoWrite "编码"项已 completed | 任一未满足 → BLOCKED，返回编码阶段 |
| evaluating → reviewing | ① evaluating 报告已生成（.opencode/harness/evidence/ 下有对应文件）② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "evaluating"项已 completed | 任一未满足 → BLOCKED，返回 evaluating 阶段 |
| reviewing → completing | ① reviewing 报告已生成（或占位符为空已记录跳过）② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "reviewing"项已 completed | 任一未满足 → BLOCKED，返回 reviewing 阶段 |

## completing 阶段 Self-Check

在刷新 state status=completed 前，**回溯验证**所有阶段已实际执行：

1. Read state.json → 确认 phase 曾经过 evaluating（current_skill 历史含 "evaluator"）
2. 确认 evaluating 报告存在（`.opencode/harness/evidence/code-evaluator-agent-review.json`）
3. 确认 reviewing 报告存在（或占位符为空已记录跳过）
4. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## success_criteria

- [ ] Plan 中所有 task 已编码实现
- [ ] evaluating 阶段已执行（harness-code-evaluator 已调用）
- [ ] reviewing 阶段已执行（或占位符为空已跳过）
- [ ] state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（所有阶段报告存在）
