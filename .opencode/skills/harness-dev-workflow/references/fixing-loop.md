# Fixing 阶段

任何 evaluating / incremental_reviewing / full_reviewing 失败时进入:

## 修复阶段技能

| 技能 | 用途 |
|------|------|
| fix-self-check | 修复环节强制自检：修前因果链诊断+爆炸半径论证，修后回归检测+健康趋势验证 |

## 修复流程

1. **读报告**提取 HIGH issue
2. **记录修复尝试**:`attempt_counts[scope_key]` 递增 + `error_signature` + `strategies_tried`
   - `scope_key`:state 的 `plan` 字段非空则为该 plan 路径;否则 `"global"`
3. **加载上方表格中的修复技能**执行修前检查:因果链诊断 + 爆炸半径论证 + 策略验证。不通过 → 再思考（回退/换策略/拆分），不人工
4. **修复代码**(每次必须换策略;禁相同代码重试)
5. **加载上方表格中的修复技能**执行修后检查:回归检测 + diff 质量 + 健康趋势。不通过 → 最小粒度回退 + 再思考
6. **重跑失败阶段**,刷新 state: `phase = trigger_stage`, `fixing = null`
7. Max 5 次 → `status = "blocked"`,写入 `.opencode/harness/blocked-reports/{module}-{timestamp}.md`(含:失败阶段、已试策略列表、error_signature、剩余 HIGH issues、建议的人工介入方向),workflow 终止退出

> BLOCKED 终止不调 `question()` 等待。用户审阅 blocked-reports 后用显式命令 resume。
> 修复技能是修复思维方式的门禁,不替代 retry 计数和 state 管理(那些由本 skill 负责)。修复技能只管 agent 怎么想,不管怎么计数。
