---
name: reverse-analysis-test-spec
description: >-
  从已有代码库逆向提取行为规格,生成 Given/When/Then 测试场景文档,作为后续测试生成的金标准。
  触发词:逆向提取 spec、从代码生成测试场景、行为规格提取、
  当用户提到"从源码提取 spec"、"给旧代码出测试场景"、"迁移/重构前准备验收标准"、
  "characterization test"、"逆向规约"、"代码行为快照"时必须使用此 skill。
---

# reverse-analysis-test-spec

## 职责

**做什么:**
- 从已有代码库逆向提取行为规格,以**契约(Contract)**和**属性(Property)**为核心提取单元
- 通过 5+1 阶段方法论,产出经过证伪精炼的冻结规格文档
- 生成 Given/When/Then 场景作为契约/属性的**见证(Witness)**,而非规格主体

**不做什么:**
- ❌ 不生成测试代码(由 test-writer agent、BDD 框架、或手工补测完成)
- ❌ 不写实现骨架、不重构、不翻译
- ❌ 不运行测试、不执行任何被测代码
- ❌ 不修改 spec 文件(一旦生成即视为冻结)

**与其他 skill / agent 的分工:**

| 角色 | 职责 |
|------|------|
| **本 skill(reverse-analysis-test-spec)** | 拐点识别 → 契约提取 → 属性发现 → 证伪精炼 → 产出冻结 spec(金标准) |
| test-writer / BDD framework | 依据 spec 生成可执行测试 |
| parity-verifier / failure-resolver | 验证迁移后的代码是否满足 spec |
| c-translate-to-rust(项目级 skill) | 翻译过程中的语言映射规则 |

---

## 适用场景

- 代码迁移 / 翻译项目:锁定源端行为预期,作为目标端的验收标准
- 大型重构前:冻结现有行为,重构后用于回归验证
- 遗留系统补测:给历史代码补测试场景,即使测试实现晚于场景设计
- 理解陌生代码库:通过提取契约与属性快速掌握代码库"保证什么"
- 任何"规格已在代码中固化,需要显式写出来"的情况
- 任何需要"从代码提取规格,供下游(translation/TCK/regression)使用"的项目

---

## 输入参数

| 参数 | 必填 | 说明 |
|------|:----:|------|
| `source_dir` | ✅ | 源代码目录路径 |
| `source_lang` | ✅ | 源代码语言标识(`c` / `java` / `rust` 等) |
| `spec_dir` | ✅ | 输出 spec 文件的目录(一般为 `spec/` 或 `.specs/`) |
| `scope` | ❌ | 限制提取范围:单模块 / 单文件 / 全量 |
| `migration_scope_hint` | ❌ | 迁移范围提示(哪些模块将被改动),用于拐点优先级排序 |
| `domain_invariants_file` | ❌ | 用户提供的领域不变量声明文件路径(若存在则整合进 spec) |

---

## 输出

`{spec_dir}/` 目录下产出的固定文件,一旦生成即视为**冻结金标准**,不应被后续任何流程修改:

| 文件 | 内容 | 来源阶段 |
|------|------|---------|
| `contracts.md` | 提取的契约(pre/post/invariant) — **主要规格** | Phase 2,经 Phase 4 精炼 |
| `properties.md` | 发现的属性(Hughes 5 类) — **次要规格** | Phase 3,经 Phase 4 精炼 |
| `state_model.md` | 状态机模型(如适用) | Phase 5 |
| `witnesses.md` | Given/When/Then 场景作为契约/属性的示例见证 | 贯穿各阶段 |
| `gaps.md` | 未被证伪也未被证明的声明,需人类判断 | Phase 4 |
| `falsification_log.md` | 证伪阶段中削弱或移除的声明及其证据 | Phase 4 |
| `coverage_report.md` | 方法论对齐的 7 项覆盖度指标 | 门禁阶段 |
| `traceability.md` | 规格声明 → 源码位置映射 | 贯穿各阶段 |
| `pending_human_input.md` | 真值锚定待人工确认项 | Phase 6 |
| `domain_invariants.md` | 用户提供或跨阶段归纳的不变量 | 贯穿各阶段 |

**冻结机制:**
- 文件写完后标记为只读(文件系统级或 git attribute)
- 后续流程(test-writer / parity-verifier)只读不写
- 如需更新 spec,必须显式重新调用本 skill,并记录变更日志

---

## 核心方法论

本 skill 的提取逻辑基于业界成熟的逆向规格提取方法论,而非场景枚举。

### 方法论基础(5 个来源,1 个综合流程)

| 来源 | 核心贡献 | 在 skill 中的角色 |
|------|---------|------------------|
| **Design by Contract** (Meyer) | 契约三元组 pre/post/invariant | Phase 2 提取格式 |
| **Property-Based Testing** (Hughes) | 5 种属性发现启发式 | Phase 3 提取启发 |
| **Daikon / Houdini** | 生成→证伪→存活者成为规格 循环 | Phase 4 精炼机制 |
| **Abstract State Machines** (Gurevich) | 状态机作为一等建模对象 | Phase 5 状态提取 |
| **Feathers / Characterization Testing** | 拐点原则:从变化处入手 | Phase 1 范围划定 |
| **Reversa** (2026) | 置信度评分 + 缺口感知 | 贯穿每个提取产出 |

### 核心反模式(禁止)

| ❌ 反模式 | ✅ 本 skill 做法 |
|-----------|-----------------|
| "枚举每个 public API 的场景" | "从拐点出发提取契约" |
| "枚举每条 if/switch 的场景" | "提取属性,然后用代码证据证伪" |
| "Given/When/Then 为 spec 主要格式" | "契约 + 属性为主,Given/When/Then 是见证" |
| "产出即视为真理" | "产出是候选,必须经过证伪才成为规格" |
| "所有路径都要覆盖" | "拐点优先,证据驱动,允许有 gaps 但必须记录" |

### 产出层级

1. **Contracts**(声明式) — "该 API 保证什么"
2. **Properties**(全称量化) — "在所有合法输入下,什么始终成立"
3. **State Model**(状态机) — "系统在状态空间上如何演化"
4. **Witnesses**(Given/When/Then) — "具体输入/输出示例,作为契约/属性的见证"

Witnesses 不是 spec 的主体,而是辅助人类读者理解契约/属性的示例。

---

## 提取流程

按顺序执行,每个阶段产出 spec 文件的一部分。

### Phase 1: 拐点识别(Inflection Point Discovery)

> 理论基础:Feathers — 不从代码顶部均匀阅读,而是从**行为变化的边界**入手。

**目的**: 不匀质扫描代码;从拐点(行为发生质变的位置)出发划定提取范围。

**输入**: `source_dir`, `migration_scope_hint`(可选)
**输出**: inflection_point_map(拐点图)、优先级排序

**步骤:**

1. **识别公共 API 表面**: 导出符号、接口定义、模块暴露的函数/方法。这些是系统与外部世界的契约点
2. **识别状态边界**: 状态被创建、修改、持久化的位置(如一个消息队列的 enqueue/dequeue 操作改变队列内部状态)
3. **识别 I/O 边界**: 磁盘读写、网络通信、硬件交互的位置。这些是系统不可见效应的观察点
4. **迁移影响排序**: 若 `migration_scope_hint` 存在,优先标记将被迁移修改的拐点 — 这些拐点的契约提取紧迫度最高
5. **构建拐点根调用图**: 以每个拐点为根节点构建调用图,而非构建一张全量平坦调用图

**拐点图示例(中性领域 — 文件解析器):**

```
拐点 IP-01: parse(input: &[u8]) -> Result<Document, ParseError>
  ├─ validate_header()        [状态边界: 校验状态写入]
  ├─ read_sections()          [IO 边界: 从文件读取]
  └─ build_document()
      └─ resolve_references() [状态边界: 解析符号表]

拐点 IP-02: serialize(doc: &Document) -> Vec<u8>
  ├─ write_header()           [IO 边界: 写入头部]
  └─ write_sections()         [IO 边界: 逐节写入]
      └─ compute_checksum()   [纯计算,无边界效应]
```

### Phase 2: 契约提取(Contract Extraction)

> 理论基础:Design by Contract (Meyer) — 每个接口由前置条件、后置条件、不变量三元组精确描述。

**目的**: 对每个拐点,提取契约(pre/post/invariant),而非列举场景。

**输入**: Phase 1 拐点图
**输出**: `contracts.md`(候选契约,待 Phase 4 精炼)

**提取方法**: 对每个拐点逐条提取:
- **Precondition(前置条件)**: 调用者在进入该拐点前必须满足的条件
- **Postcondition(后置条件)**: 系统在执行完成后保证的条件(可能有多个条件后置)
- **Invariant(不变量)**: 无论走哪条执行路径,在执行前后都成立的断言

**契约格式:**

```markdown
### CONTRACT-{ID}: {拐点名称}
**Source**: `{文件路径}:{行号区间}`
**Kind**: API-boundary | State-boundary | IO-boundary
**Confidence**: high | medium | low

**Preconditions**:
1. {条件} — *evidence: `{代码引用,如 file:line 或函数名}`*
2. {条件} — *evidence: `{代码引用}`*

**Postconditions**:
1. When {触发条件}: {保证} — *evidence: `{代码引用}`*
2. When {触发条件}: {保证} — *evidence: `{代码引用}`*

**Invariants**:
1. {始终成立的断言} — *evidence: `{代码引用}`*
```

**Evidence 是强制的**: 每条条款必须引用具体代码(file:line 或函数名)。无证据的条款不允许写入契约。

**Confidence 评分标准(Reversa 风格):**

| 等级 | 含义 | 识别特征 |
|------|------|---------|
| **high** | 条款直接在代码中可见,表现为显式检查或赋值 | `assert`, `if (x == NULL) return ERR;`, 直接赋值 |
| **medium** | 条款从控制流 / 数据流推断得出 | 循环终止条件暗示不变量、函数返回值类型约束 |
| **low** | 条款由领域惯例或代码模式隐含 | 命名约定暗示语义、惯用法隐含约束 — **标记为 Phase 4 重点证伪对象** |

### Phase 3: 属性提取(Property Discovery)

> 理论基础:Hughes 的 5 种属性类型 — 系统性地发现全称量化属性,而非仅关注单次调用的契约。

**目的**: 对每个契约,用 5 个属性问题启发式地发现更深层、更广范围的行为属性。

**输入**: Phase 2 候选契约
**输出**: `properties.md`(候选属性,待 Phase 4 精炼)

**5 个属性问题(每个契约逐一提问):**

| # | 属性类型 | 启发式问题 | 中性领域示例 |
|---|---------|-----------|-------------|
| 1 | **不变式(invariant preservation)** | 该操作前后,什么被保留? | "排序后元素的 multiset 等于输入" |
| 2 | **后置条件(postcondition)** | 完成后无论输入如何,什么一定成立? | "insert 之后 contains(x) 必为 true" |
| 3 | **变质关系(metamorphic)** | 输入按已知方式变化时,输出如何变化? | "f(reverse(x)) 与 f(x) 的字节顺序相反" |
| 4 | **参考等价(model-based)** | 是否存在更简单的参考实现,本系统应与之等价? | "自定义 sort ≡ std::sort 在所有输入上" |
| 5 | **归纳(inductive)** | 是否有基础情形 + 递推扩展? | "空集 + 增量添加 = 完整集合" |

**并非每个契约都适用全部 5 类** — 不适用时,显式标注跳过原因。

**属性格式:**

```markdown
### PROPERTY-{ID}: {简短名称}
**Type**: invariant | postcondition | metamorphic | model-based | inductive
**Applies to**: {合约 ID 列表}
**Statement**: ∀ inputs meeting {precondition}, {property holds}
**Confidence**: high | medium | low
**Evidence**: `{代码引用}`

**Skipped types**: {跳过的属性类型及原因,如 "model-based: 无简单参考实现"}
```

### Phase 4: 证伪与精炼(Falsification & Refinement)

> 理论基础:Daikon/Houdini — 生成的候选不变量通过代码证据进行 generate→check→keep 循环,只有经受住证伪的才成为规格。

**目的**: Phase 2 和 Phase 3 产出的每一条契约和属性都是**候选**;本阶段试图用代码证据证伪它们。未被动摇的候选升格为冻结规格。

**证伪机制(组合使用):**

1. **逻辑矛盾检查**: 代码中是否存在一条执行路径,直接违反所声称的契约/属性?
2. **边缘用例探索**: 能否构造一个边界输入,使该属性不成立?
3. **错误路径分析**: 错误路径(catch/early return)是否破坏了不变量?若是,不变量需加条件限定
4. **状态变异追踪**: 两个契约点之间的状态修改是否打破了所声称的不变量?

**每条候选声明的判定结果:**

| 结果 | 含义 | 处理 |
|------|------|------|
| **Survived** | 未找到证伪证据 | 升格为冻结规格,置信度维持不变 |
| **Falsified** | 代码证据直接矛盾 | 移除该声明,记录在 `falsification_log.md` |
| **Weakened** | 部分证伪,需增加前置条件才成立 | 精炼后保留,记录削弱证据 |
| **Unknown** | 证据不足以判定 | 记录在 `gaps.md`,加入 `pending_human_input.md` |

**输出:**
- 冻结的 contracts + properties(经受住证伪或被精炼后的版本)
- `falsification_log.md` — 被削弱或移除声明的证伪证据
- `gaps.md` — 无法证伪也无法证明的声明
- `pending_human_input.md` — 需人类判断的项目(延续 Phase 6)

### Phase 5(可选): 状态模型提取(State Model Extraction)

> 理论基础:Abstract State Machines (Gurevich) — 有状态系统应建模为状态机,而非散落在各场景中的状态转换标签。

**触发条件**(满足任一即触发):
- 代码中存在状态枚举(如 `enum State { Init, Running, Stopped }`)
- 多个函数修改同一字段/状态变量
- 系统具有生命周期阶段(初始→运行→关闭)

**目的**: 将有状态行为建模为状态机,作为一等规制对象。

**输出**: `state_model.md`(独立文件)

```markdown
## State Model: {系统名称}

**States**: {枚举值及描述}
**Initial state**: {初始状态}

**Transitions**:
| From | Event(触发操作) | To | Guard(前置条件) | Effect(后置条件) |
|------|----------------|----|-----------------|-----------------|
| A | op_x() | B | {前置条件} | {后置条件} |

**Illegal transitions**: 代码中未观察到的 (from, to) 对 — 可能是 bug 或不可能路径

**Cross-state invariants**: {在所有状态下都成立的断言}
```

状态模型是**一等产出**,不是标注了 Kind=state-transition 的散落场景。

### Phase 6(可选): 真值锚定(Truth Anchoring)

**触发条件**: 契约/属性的 Then-值无法从代码静态读取(如序列化后的字节序、密码学输出、运行时浮点计算结果)。

**目的**: 确保金标准中的具体数值来自参考实现的真实运行结果,而非人工推测。

**输出**: `pending_human_input.md` — 列出所有需要人工运行参考代码才能填入的具体值。

详见 [references/truth-anchoring.md](references/truth-anchoring.md)。

---

## 规格格式规范:Given/When/Then 见证格式

Given/When/Then 在本 skill 中的角色是**见证(Witness)**,用于为契约和属性提供具体示例,帮助人类读者理解抽象的声明式规格。Witnesses 是辅助性的,不是规格的载体。

### 标准结构

```markdown
### WITNESS-{ID}: {简短标题}

**Attests to**: {所见证的 CONTRACT-ID 或 PROPERTY-ID}
**Source ref**: `{源文件路径}:{行号区间}`

**Given** {前置条件 — 系统状态 + 输入数据}
- 条件 1
- 条件 2

**When** {触发操作 — 调用 API 或执行操作}
- 操作 1

**Then** {可观察断言 — 必须可验证}
- 断言 1(具体值、具体类型、具体状态)
- 断言 2

**Witness kind**: happy-path | edge-case | error-path | boundary | state-transition | format-assertion
```

### 6 种 Witness Kind(仅适用于见证)

| Kind | 说明 | 典型示例 |
|------|------|---------|
| happy-path | 正常执行路径的见证 | 一个消息队列 enqueue 后 dequeue 返回相同消息 |
| edge-case | 边缘输入的见证 | 空输入、极大值、重复操作 |
| error-path | 错误路径的见证 | 无效输入触发特定错误返回 |
| boundary | 边界条件的见证 | 容量上限、最大长度、对齐要求 |
| state-transition | 状态转换的见证 | 状态机中从 A 到 B 的具体触发 |
| format-assertion | 格式/字节级的见证 | 序列化输出的具体字节模式 |

### Then 断言可验证性

**每条 Then 断言必须可验证。** 以下形式无效:

| ❌ 无效 | ✅ 有效 |
|--------|--------|
| "返回正确值" | "返回值 == 0" 或 "返回 `Err::NotFound`" |
| "数据库已更新" | "调用 `get(key='x')` 返回 `'y'`" |
| "性能良好" | "1MB 输入处理时间 < 100ms" |
| "不抛异常" | "不抛异常,且返回值 == Ok(())" |

---

## References

| 文件 | 用途 | 不可删理由 |
|------|------|---------|
| [references/methodology-reference.md](references/methodology-reference.md) | 方法论详细背景(Daikon、DbC、Hughes、ASM、Reversa、Feathers) | 理解 "why" — 需要深入了解方法论基础时使用 |
| [references/specification-templates.md](references/specification-templates.md) | 契约、属性、状态模型、Witness 模板 | LLM 写规格时容易格式漂移,需要模板锚定 |
| [references/coverage-checklist.md](references/coverage-checklist.md) | 7 项方法论指标的机械验证方法 | LLM 容易产生"自以为完整"的错觉,需要客观计数核对 |
| [references/truth-anchoring.md](references/truth-anchoring.md) | Phase 6 真值锚定方法论 | LLM 不会主动思考"金标准本身可能错" |

---

## 验证门禁(spec 交付前必须通过)

| # | 指标 | 公式 | 目标 |
|---|------|------|------|
| **A** | 契约完备性 | `\|有源码引用的契约\| / \|拐点数\|` | 100% |
| **B** | 属性覆盖度 | `\|有 ≥1 属性的契约\| / \|契约总数\|` | ≥ 70% |
| **C** | 证据锚定率 | `\|有代码引用的条款\| / \|条款总数\|` | 100% |
| **D** | 证伪覆盖度 | `\|进入 Phase 4 且完成判定的声明\| / \|进入 Phase 4 的声明总数\|` | 100% |
| **E** | 缺口文档化 | `\|在 gaps.md 中有理由的声明\| / \|gaps.md 中的声明总数\|` | 100% |
| **F** | 见证覆盖度 | `\|有 ≥1 witness 的契约+属性\| / \|契约+属性总数\|` | ≥ 50% |
| **G** | 真值锚定率 | `\|已填入真实值的项\| / \|需真值锚定的项总数\|` | 0%(未启用) 或 100%(启用 Phase 6) |

任一门禁未通过:在 `coverage_report.md` 中标记未通过的项,并向调用方报告缺口,而非默默降低标准。

---

## 禁止事项

1. ❌ 修改被测代码(本 skill 只读源码,不写源码)
2. ❌ 在 Then 中使用"正确"、"合适"、"良好"等不可验证词汇
3. ❌ 生成测试代码、实现骨架、或任何可执行内容
4. ❌ 运行被测代码(真值锚定由 Phase 6 的隔离环境完成,不在主流程)
5. ❌ 在 spec 交付后再次修改 spec(重新提取需要显式重新调用本 skill)
6. ❌ 把"理解到的意图"写进 spec(只提取代码实际做的事)
7. ❌ 跳过任何 Phase(可标注"不适用",但必须显式说明并记入 coverage_report.md)
8. ❌ **在没有 Phase 4 证伪的情况下声称契约/属性成立**
9. ❌ **在没有代码证据的情况下写出契约条款**(evidence 引用是强制的)
10. ❌ **把场景(scenario)作为 spec 主体**(Given/When/Then 只能是见证)
11. ❌ **跳过 Phase 1 直接从代码顶部开始枚举**(必须从拐点入手)
12. ❌ 证伪失败的条款不允许静默丢弃(必须记录在 falsification_log.md 或 gaps.md 中)

## 强制事项

1. ✅ 每个契约/属性条款必须附代码证据(file:line 或函数引用)
2. ✅ 每个契约/属性条款必须标注置信度(high/medium/low)
3. ✅ coverage_report.md 必须包含 7 项方法论指标(A-G)的检查结果
4. ✅ 证伪失败的条款必须明确标注为 gap,不允许静默丢弃
5. ✅ 所有 spec 文件在各 Phase 完成后视为冻结
6. ✅ traceability.md 必须覆盖每条拐点 → 源码位置映射
7. ✅ 跳过 Phase 必须在 coverage_report.md 中说明原因(如"源码无状态枚举,跳过 Phase 5")
8. ✅ 无法立即填写的期望值必须追加到 `pending_human_input.md`,禁止硬编 AI 推测
9. ✅ falsification_log.md 必须记录每条被削弱或移除的声明及其证伪证据
10. ✅ gaps.md 中每条声明必须附"为何无法判定"的理由

---

## 快速参考:执行步骤

```
 1. Phase 1: 拐点识别 → 拐点图 + 优先级排序
 2. Phase 2: 契约提取 → contracts.md(候选)
 3. Phase 3: 属性提取 → properties.md(候选)
 4. Phase 4: 证伪与精炼 → 冻结 contracts + properties;产出 gaps.md + falsification_log.md
 5. Phase 5(可选): 状态模型 → state_model.md
 6. Phase 6(可选): 真值锚定 → pending_human_input.md
 7. 跑覆盖度门禁(A-G)→ coverage_report.md
 8. 标记 spec_dir/ 只读
 9. 输出短摘要
```
