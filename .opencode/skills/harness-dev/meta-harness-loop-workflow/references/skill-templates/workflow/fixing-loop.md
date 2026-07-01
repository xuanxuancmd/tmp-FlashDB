# Fixing 阶段（通用修复循环权威）

Fixing 由**主 Agent** 编排：拉起 executor(mode=fix) 修复 → 拉起 checker sub-agent 重新验证。本文件是修复循环的通用权威定义，workflow 的 2.3/3.3 和 review/evaluator skill 均引用本文件，仅指定实例参数（max_rounds / checker 类型 / worktree_path）。

## 触发条件

- 检视阶段报告 `pass=false` → 检视修复循环（实例 `max_rounds=3`, `checker=code-review-agent`）
- 评估阶段报告 `pass=false` → 评估修复循环（实例 `max_rounds=5`, `checker=code-evaluator-agent`）

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
│  ② 【修前】加载 fix-self-check skill（若 {fixing_skills} │
│     非空），执行修前检查（因果链诊断 + 爆炸半径论证）       │
│      ↓                                                  │
│  ③ task(executor, mode=fix, issues=blocking_issues,    │
│         context_summary=SUMMARY路径,                    │
│         worktree_path={该 Plan 的 worktree})            │
│      → 等待返回 status=completed                        │
│      ↓                                                  │
│  ④ 【修后】加载 fix-self-check skill（若 {fixing_skills} │
│     非空），执行修后检查（回归检测 + diff 质量）           │
│      ↓                                                  │
│  ⑤ 重新拉起 checker sub-agent:                          │
│     - 检视修复: task(code-review-agent, review_path)   │
│     - 评估修复: task(code-evaluator-agent, plan_path)  │
│      → 等待返回报告路径                                  │
│      ↓                                                  │
│  ⑥ 读 checker 报告（见"evidence 消费原则"）:             │
│     pass=true → 修复完成,退出循环 ✅                     │
│     pass=false → 提取 blocking_issues → 返回循环开始     │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

## evidence 消费原则

### 原则1：summary.json 是决策入口

先 Read checker 的 summary/json 报告：
- `overall_result.pass=true` → 修复完成 ✅
- `overall_result.pass=false` → 从 `blocking_issues[]` 提取阻断性问题，继续循环

### 原则2：按需读取 evidence 文件

summary.json 中每个检查项含 evidence_file 路径。当检查项 `pass=false` 时，Read 对应 evidence 文件定位具体问题：

| 检查项失败 | 读取文件 | 处理方式 |
|-----------|---------|---------|
| build_check | build-result.txt | 定位编译错误，传给 executor(mode=fix) |
| test_check | test-result.txt | 定位失败用例，传给 executor(mode=fix) |
| AI 检查 | ai-check-*.json / ai-diff.json | 定位具体问题，传给 executor(mode=fix) |
| placeholder_detection | placeholder.json | 传给 executor(mode=fix) 补全 |

### 原则3：report.md 作为补充阅读

Read `<module>-review-report.md` 或 `code-evaluator-agent-review.md` 获取详细分析、修复建议（人阅读视角，辅助主 Agent 理解问题）。

## blocking_issues 分类处理

主 Agent 从 checker 报告提取 issues 后，按严重度分类传给 executor(mode=fix)：

| 严重度 | 类型 | 传给 executor 的修复指令 |
|--------|------|---------|
| **HIGH** | missing | issue 标注"缺失实现"，executor 补充 |
| **HIGH** | incomplete | issue 标注"逻辑不完整"，executor 补全 |
| **HIGH** | wrong | issue 标注"实现有误"，executor 对照原始需求重写 |
| **HIGH** | unresolved | issue 标注"未触及根因"，executor 分析根因后修复 |
| **HIGH** | regression | issue 标注"引入回归"，executor 回退或修复回归 |
| **MEDIUM** | 任意 | 主 Agent 评估后决定：可接受则跳过，否则传给 executor 修复 |

**优先级规则**：HIGH 阻断性优先处理，修复期间跳过非阻塞工作。

## 主 Agent 在修复循环中的职责

1. **提取 issues**：从 checker 的 summary.json/JSON 报告提取 `blocking_issues[]`（结构化 JSON）
2. **传递上下文**：将上一轮编码/修复的 SUMMARY 路径传给 executor(mode=fix)，供其理解上下文
3. **传递 worktree_path**：多 Plan 模式下，确保 executor 和 checker 都在正确的 worktree 中工作
4. **记录修复尝试**：刷新 state 的 `attempt_counts[plan].count` 递增 + `error_signature` + `strategies_tried`
5. **加载 fix-self-check skill（若 {fixing_skills} 非空）**：主 Agent 在决策修复策略时加载，执行修前检查（因果链+爆炸半径）+ 修后检查（回归+趋势）

## fix-self-check 适用说明

fix-self-check 是修复思维方式的门禁，指导主 Agent **如何决策修复方向**，不替代 retry 计数和 state 管理。

主 Agent 在以下时机加载 fix-self-check：
- **修前**（步骤 ②）：提取 issues 后、拉起 executor(mode=fix) 前 — 因果链诊断 + 爆炸半径论证
- **修后**（步骤 ④）：executor 返回后、重新拉起 checker 前 — 回归检测 + diff 质量

> fix-self-check 只管主 Agent 怎么想，不管怎么计数。不通过 → 最小粒度回退 + 再思考，不人工。

## 终止处理

达到 max_rounds 仍未通过时，设该 Plan `status=blocked`：

1. 剩余 HIGH 阻断性问题列表（含位置和描述）写入 evidence 报告
2. 剩余 MEDIUM 非阻断性问题汇总写入 evidence 报告
3. 最后一次 checker 报告路径记录在 state.json 的 `blocked_reason`
4. state.json 刷新为 `status=blocked`，由终态机制触发人工介入

> **BLOCKED 终止不调 `question()` 等待**。用户审阅 blocked-reports 后用显式命令 resume。
