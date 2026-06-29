# Coverage Checklist

> 返回 [SKILL.md](../SKILL.md)

---

## 7 项方法论指标

| # | 指标 | 公式 | 目标 | 数据来源 |
|---|---|---|---|---|
| **A** | Contract Completeness | `|contracts with source ref| / |inflection points|` | 100% | 机械计数 |
| **B** | Property Coverage | `|contracts with ≥1 property| / |contracts|` | ≥ 70% | 机械计数 |
| **C** | Evidence Grounding | `|clauses with code citation| / |all clauses|` | 100% | 机械检查 |
| **D** | Falsification Coverage | `|claims through Phase 4| / |claims entering Phase 4|` | 100% | 对账 falsification_log.md |
| **E** | Gap Documentation | `|gaps with reason| / |gaps.md entries|` | 100% | 机械检查 |
| **F** | Witness Coverage | `|claims with ≥1 witness| / |claims|` | ≥ 50% | 机械计数 |
| **G** | Truth Anchoring Completion | `|filled pending items| / |total pending|` | 0% 或 100% | 机械统计 |

**指标 A–F 全部强制**。任一项未达标 → 不允许进入下游(test-writer / parity-verifier)。G 仅在启用 Phase 6 时检查。

---

## 指标 A: Contract Completeness — 机械计数

**含义**:每一个在 Phase 1 识别的拐点,都必须在 `contracts.md` 中有至少一个带源码引用的 Contract。

**计数步骤**:

1. **统计拐点总数**:从拐点清单(Phase 1 产出)中计数。

   ```powershell
   # 从拐点清单中统计唯一拐点 ID
   (Select-String -Path "spec/inflection_points.md" -Pattern "^### IP-" |
       Measure-Object).Count
   ```

2. **统计有 Contract 的拐点数**:从 `contracts.md` 中统计有 `Source:` 引用的 Contract,比对拐点 ID。

   ```powershell
   # 统计 contracts.md 中带 Source 引用的 Contract
   (Select-String -Path "spec/contracts.md" -Pattern "^### CONTRACT-\d+" |
       Measure-Object).Count
   ```

3. **比对**:Contract 数 >= 拐点总数 → 100%。缺少 → 记录到 `gaps.md` 并附原因。

**缺口处理**:如果某个拐点没有任何 Contract(可能是因为该拐点的行为不可静态推断),必须:
- 在 `gaps.md` 中记录该拐点
- 给出 reason(如:"该拐点的行为依赖运行时配置,无法提取静态契约")
- 在 `coverage_report.md` 中标注为 "gap: {reason}"

---

## 指标 B: Property Coverage — 机械计数

**含义**:大部分 Contract 应该至少被一个 Property 补充 — Property 是 Contract 的横切补充,捕捉跨操作的全局性质。

**计数步骤**:

1. **从 `properties.md` 中提取 `Applies to:` 字段**,得到被 Property 覆盖的 Contract 集合。

   ```powershell
   # 统计 properties.md 中引用的唯一 CONTRACT-ID
   (Select-String -Path "spec/properties.md" -Pattern "Applies to:\s+(CONTRACT-\d+)" |
       ForEach-Object { $_.Matches[0].Groups[1].Value } |
       Sort-Object -Unique |
       Measure-Object).Count
   ```

2. **比对**:被覆盖的 Contract 数 / Contract 总数 >= 70% → 达标。

**为什么目标 ≥ 70% 而非 100%?**  
有些 Contract 可能没有有意义的补充 Property — 例如一个纯粹的数据转换 Contract 的全部语义已经被 pre/post/invariant 完整描述,不需要额外的 invariant-preservation 或 metamorphic Property。但 70% 确保我们不是在"只写 Contract 不做 Property 发现"。

---

## 指标 C: Evidence Grounding — 机械检查

**含义**:contracts.md 和 properties.md 中的**每一个条款**(precondition、postcondition、invariant、property statement)都必须有代码引用。

**计数步骤**:

1. **提取所有条款**:从 contracts.md 和 properties.md 中提取编号条款(如 `1.`, `2.`)。

2. **检查 evidence 引用**:每个条款行必须包含 `evidence:` 关键词或 `{file}:{line}` 格式引用。

   ```powershell
   # 检查 contracts.md 中的条款是否都有 evidence 引用
   $clause_lines = Select-String -Path "spec/contracts.md" -Pattern "^\d+\."
   $no_evidence = $clause_lines | Where-Object {
       $_.Line -notmatch "evidence:" -and $_.Line -notmatch "\w+\.\w+:\d+"
   }
   $no_evidence.Count  # 应为 0
   ```

3. **properties.md 同样检查**:

   ```powershell
   # 检查 properties.md 的 Statement 和 Evidence 字段
   $no_ev = Select-String -Path "spec/properties.md" -Pattern "^(\*\*Statement\*\*|\*\*Evidence\*\*):" |
       Where-Object { $_.Line -notmatch "\w+\.\w+:\d+" }
   ```

4. **判定**:无 evidence 的条款数 == 0 → 100%。

---

## 指标 D: Falsification Coverage — 对账

**含义**:进入 Phase 4 证伪循环的每一个候选声明(Contract 条款 + Property),都必须经过证伪检查并有明确结果。

**对账方法**:

1. **统计进入 Phase 4 的声明总数**:
   - `contracts.md` 中的条款总数(preconditions + postconditions + invariants)
   - `properties.md` 中的 Property 总数
   - (如果启用了 Phase 5: `state_model.md` 中的声明)

2. **统计已处理的声明**:以下三类声明都算"已处理":
   - **Survived**:在 falsification_log.md 中标记为 SURVIVED(有记录显示它经受了证伪尝试)
   - **Weakened**:在 falsification_log.md 中标记为 WEAKENED(原始声明被弱化,但弱化后的版本存活)
   - **Removed**:在 falsification_log.md 中标记为 REMOVED(被证伪,已删除)

   ```powershell
   # 统计 falsification_log.md 中的处理条目
   (Select-String -Path "spec/falsification_log.md" `
       -Pattern "(SURVIVED|WEAKENED|REMOVED)" |
       Measure-Object).Count
   ```

3. **比对**:已处理声明数 == 进入 Phase 4 的声明总数 → 100%。

4. **缺口**:如果有声明既没有 survived、也没有 weakened、也没有 removed → 它跳过了证伪,必须补做。

---

## 指标 E: Gap Documentation — 机械检查

**含义**:gaps.md 中的每一个条目都必须有 reason 字段,说明为什么该信息无法提取。

**计数步骤**:

```powershell
# 统计 gaps.md 中的条目(## 标题)
$total_gaps = (Select-String -Path "spec/gaps.md" -Pattern "^## " |
    Measure-Object).Count

# 统计有 Reason: 字段的条目
$with_reason = (Select-String -Path "spec/gaps.md" -Pattern "^\*\*Reason\*\*:" |
    Measure-Object).Count

# 比对
$with_reason / $total_gaps  # 应为 1.0 (100%)
```

**判定**:二值 — 每个 gap 都有 reason = 通过;任一缺失 = 不通过。

---

## 指标 F: Witness Coverage — 机械计数

**含义**:主规格(contracts.md + properties.md + state_model.md)中的声明,至少 50% 应该有一个 Witness 示例。

**计数步骤**:

1. **统计主规格声明总数**:
   - contracts.md 条款数 + properties.md Property 数 + state_model.md 转换数

2. **统计有 Witness 的声明**:从 `witnesses.md` 中提取 `Witnesses:` 字段引用的 ID。

   ```powershell
   # 统计 witnesses.md 中引用的唯一 claim ID
   (Select-String -Path "spec/witnesses.md" -Pattern "Witnesses:\s+(CONTRACT-\d+|PROPERTY-\d+)" |
       ForEach-Object { $_.Matches[0].Groups[1].Value } |
       Sort-Object -Unique |
       Measure-Object).Count
   ```

3. **比对**:有 Witness 的声明数 / 主规格声明总数 >= 50% → 达标。

**为什么 50% 而非 100%?**  
不是每个声明都需要 Witness — 一些简单的前置条件(如 "指针非空")用 Contract 就足够清晰,写 Witness 反而冗长。但 50% 确保主规格有足够的示例性文档。

---

## 指标 G: Truth Anchoring Completion — 机械统计

**含义**:如果启用了 Phase 6(真值锚定),`pending_human_input.md` 中的所有待补充项必须被回填或删除(说明不需要锚定)。

**计数步骤**:

```powershell
# 统计 pending_human_input.md 中的待补充项
$total = (Select-String -Path "spec/pending_human_input.md" -Pattern "^## " |
    Measure-Object).Count

# 统计已填入的(有"填入:"且内容非空)
$filled = (Select-String -Path "spec/pending_human_input.md" -Pattern "^- 填入:\s+\S+" |
    Measure-Object).Count

# 两种合法状态:
# 1. $total == 0 (Phase 6 未启用,或所有项已回填后文件被删除)
# 2. $filled == $total (100% 完成)
```

**判定**:只在 Phase 6 启用时检查。目标为 0%(文件不存在 = Phase 6 未启用)或 100%(所有 pending 项已回填)。中间值不允许 — 要么全部完成,要么不启用。

---

## 缺口处理(任一门禁未通过)

1. **显式记录缺口**到 `coverage_report.md`(指标字母 + 当前值 + 目标值)
2. **评估原因**:
   - 指标 A 缺口:拐点没有被 Contract 覆盖 → 补充 Contract 或记录到 `gaps.md`
   - 指标 B 缺口:某些 Contract 没有 Property → 重新对每个 Contract 问 Hughes 5-question
   - 指标 C 缺口:条款缺少 evidence → 补充代码引用,或删除无据条款(记录到 falsification_log.md)
   - 指标 D 缺口:声明未经证伪 → 回到 Phase 4 补做
   - 指标 E 缺口:gap 缺少 reason → 补充 reason
   - 指标 F 缺口:声明缺少 Witness → 补充 Witness 或确认该声明不需要示例
   - 指标 G 缺口:pending 项未回填 → 完成回填或标记为不需要
3. **重跑全部门禁**(不是只跑补救那一项,防止回归)
4. **全部通过 → 标记 spec 文件为 frozen**;否则报告缺口,由人工决策

## 禁止

- ❌ 跳过任一门禁而不记录
- ❌ 把"无意义的契约"从总数中减掉来美化覆盖率
- ❌ 只跑单项门禁(必须全部重跑)
- ❌ 接受无 evidence 引用的 claims(指标 C 必须 100%)
- ❌ 在 `falsification_log.md` 中遗漏任何声明的证伪记录
- ❌ 将 gaps.md 中的条目不计入缺口统计
