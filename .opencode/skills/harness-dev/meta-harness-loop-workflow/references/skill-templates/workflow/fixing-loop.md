# Fixing 阶段

Fixing 由**主 Agent** 编排:拉起 executor(mode=fix) 修复 → 拉起 checker sub-agent 重新验证。

## 触发条件

- 检视阶段(Phase 2)报告 `pass=false` → 检视修复循环(max 3 轮)
- 评估阶段(Phase 3)报告 `pass=false` → 评估修复循环(max 5 轮)

## 修复循环流程

```
round = 0

┌─ 循环开始 ─────────────────────────────────────────────┐
│                                                         │
│  round >= max_rounds ?                                  │
│      ├─ YES → 该 Plan status=blocked（见"终止处理"）     │
│      └─ NO  → round += 1                                │
│                                                         │
│  ① 从上一步 checker 报告提取 blocking_issues[]           │
│      ↓                                                  │
│  ② task(executor, mode=fix, issues=blocking_issues,    │
│         context_summary=SUMMARY路径,                    │
│         worktree_path={该 Plan 的 worktree})            │
│      → 等待返回 status=completed                        │
│      ↓                                                  │
│  ③ 重新拉起 checker sub-agent:                          │
│     - 检视修复: task(code-review-agent, review_path)   │
│     - 评估修复: task(code-evaluator-agent, plan_path)  │
│      → 等待返回报告路径                                  │
│      ↓                                                  │
│  ④ 读 checker 报告:                                     │
│     pass=true → 修复完成,退出循环 ✅                     │
│     pass=false → 返回循环开始                            │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

## 主 Agent 在修复循环中的职责

1. **提取 issues**:从 checker 的 summary.json/JSON 报告提取 `blocking_issues[]`（结构化 JSON）
2. **传递上下文**:将上一轮编码/修复的 SUMMARY 路径传给 executor(mode=fix)，供其理解上下文
3. **传递 worktree_path**:多 Plan 模式下，确保 executor 和 checker 都在正确的 worktree 中工作
4. **记录修复尝试**:刷新 state 的 `attempt_counts[plan].count` 递增 + `error_signature` + `strategies_tried`
5. **加载 fix-self-check skill（若 {fixing_skills} 非空）**:主 Agent 在决策修复策略时加载,执行修前检查(因果链+爆炸半径)+修后检查(回归+趋势)

## fix-self-check 适用说明

fix-self-check 是修复思维方式的门禁,指导主 Agent **如何决策修复方向**,不替代 retry 计数和 state 管理。

主 Agent 在以下时机加载 fix-self-check:
- 提取 issues 后、拉起 executor(mode=fix) 前:修前检查(因果链诊断 + 爆炸半径论证)
- executor 返回后、重新拉起 checker 前:修后检查(回归检测 + diff 质量)

> fix-self-check 只管主 Agent 怎么想,不管怎么计数。不通过 → 最小粒度回退 + 再思考,不人工。

## 终止处理

达到 max_rounds 仍未通过时,设该 Plan `status=blocked`:

1. 剩余 HIGH 阻断性问题列表(含位置和描述)写入 evidence 报告
2. 剩余 MEDIUM 非阻断性问题汇总写入 evidence 报告
3. 最后一次 checker 报告路径记录在 state.json 的 `blocked_reason`
4. state.json 刷新为 `status=blocked`,由终态机制触发人工介入

> BLOCKED 终止不调 `question()` 等待。用户审阅 blocked-reports 后用显式命令 resume。
