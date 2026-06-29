# Methodology Reference

> 返回 [SKILL.md](../SKILL.md)

本文档是"为什么这样提取"的深度参考,适合想理解设计原理的用户阅读,不是执行本 skill 所必需的。

## 方法论来源一览

| 来源 | 关键人物 | 核心思想 | 在 skill 中的角色 |
|---|---|---|---|
| Design by Contract | Bertrand Meyer | 契约三元组 pre/post/invariant,契约即规格 | Phase 2 (契约提取) |
| Property-Based Testing | John Hughes, David MacIver | 5 种属性发现启发式 | Phase 3 (属性提取) |
| Daikon / Houdini | Ernst, Flanagan, Leino | 生成→证伪→存活的循环 | Phase 4 (证伪与精炼) |
| Abstract State Machines | Yuri Gurevich, Egon Börger | 状态机作为一等建模对象 | Phase 5 (状态模型提取) |
| Characterization Testing | Michael Feathers | 拐点原则 + 测试即描述 | Phase 1 (拐点识别) |
| Reversa | 2026 multi-agent research | 置信度评分 + 缺口感知 | 贯穿全流程 |

---

## 1. Design by Contract (Meyer)

**Design by Contract (DbC)** 是 Bertrand Meyer 在 1986 年为 Eiffel 语言提出的软件工程方法论。其核心思想可以用一句话概括:**软件系统的每一个组件之间的交互,都可以且应当被描述为一组精确的契约(Contract)。** 一个契约由三个部分组成:

- **前置条件 (Precondition)**:调用方的义务 — 调用者必须保证的条件。如果前置条件不满足,被调用方可以做任何事(包括崩溃),因为调用者已经"违约"。
- **后置条件 (Postcondition)**:供应方的保证 — 如果被调用方正常返回,它保证的结果。这是供应方对调用方的承诺,无论内部实现如何变化,这个承诺必须被满足。
- **不变量 (Invariant)**:一致性约束 — 在任何公开操作开始前和结束后都必须为真的条件。它描述了对象/模块"永远是什么",而非"做了什么"。

契约**就是**规格 — 它不依赖于实现细节,实现只是满足契约的一种"秘密"。这意味着一个契约可以覆盖无限的输入空间:只要前置条件被满足,后置条件就必须成立。这比基于场景的枚举强大得多,因为场景只能覆盖有限的输入组合,而契约直接定义了整个有效输入域的行为保证。

Eiffel 中的 `old` 关键字允许后置条件引用调用前的值。例如 `count = old count + 1` 表达"调用后 count 比调用前正好多了 1"。这个概念对于逆向工程至关重要 — 当我们从代码中提取后置条件时,需要明确区分"当前状态的值"和"调用前状态的值"。

**从没有 DbC 标注的代码中提取契约,有系统的协议:**

1. **Guard clauses → 前置条件**:函数开头的参数校验、`assert!`、提前返回 `Err` 的条件,这些本质上都是前置条件的运行时检查。每一个 guard 都对应一条前置条件。
2. **Return assertions → 后置条件**:函数返回值的构造、`Ok()` 携带的状态、以及所有在函数末尾被赋值的 out-parameters,构成了后置条件。
3. **Field consistency → 不变量**:跨多个操作保持一致的字段关系(如 `len <= capacity`、`used + free == total`)是不变量候选。

Meyer 的关键洞察是:契约三元组不是"文档辅助",而是"规格本身"。如果我们能提取出契约,我们就提取出了规格的骨架 — 后续的属性、场景都是对这个骨架的补充和示例。

> **参考文献**: Meyer, B. "Applying Design by Contract." *IEEE Computer*, 25(10), 1992, pp. 40–51.

---

## 2. Property-Based Testing (Hughes / MacIver)

Property-Based Testing (PBT) 由 John Hughes 和 Koen Claessen 在 1999 年随 QuickCheck 提出,后由 David MacIver 的 Hypothesis 进一步推广。其核心哲学可以用 MacIver 的一句话概括:

> **"Example-based tests make stronger claims than they can demonstrate."**

一个例子声称"对于这个特定输入,输出是这个特定值",但它真正想说的是"对于所有满足前置条件的输入,输出都满足某个性质"。PBT 把这种隐含的普遍声明变成了显式的属性(Property)声明,然后用随机化测试来验证。

Hughes 在 2019 年的 "How to Specify It!" 中总结了**5 种属性发现启发式**,它们是发现属性的系统性方法:

1. **不变量保持 (Invariant Preservation)**:操作执行前后,什么东西保持不变?例如:排序不改变元素集合(只是重排)、解析不丢失字节、写入不改变未涉及的数据。这种属性回答的是 "什么不会被破坏"。

2. **后置条件 (Postcondition)**:操作完成后,什么条件必须成立?例如:插入后元素可查、删除后元素不可查、解析后所有字段非空。这与 DbC 的后置条件概念重叠,但 PBT 的视角更宽 — 它不局限于返回值,也可以是全局状态的任何可观测性质。

3. **形变关系 (Metamorphic)**:当输入以某种方式变换时,输出如何相应变换?例如:`sort(sort(x)) == sort(x)` (幂等性)、`reverse(sort(x)) == sort_descending(x)` (反转关系)、`encrypt(decrypt(x)) == x` (互逆性)。这种属性特别适合没有明确"正确答案"的场景 — 你不知道正确答案,但你知道答案之间应该有某种关系。

4. **基于模型 (Model-based)**:存在一个更简单的参考实现吗?例如:自定义队列的行为应当匹配 `std::VecDeque`、自定义排序的输出应当匹配标准库排序。这个属性类型是"差分测试"的理论基础 — 用已知正确的简单模型来验证复杂实现。

5. **归纳 (Inductive)**:操作是否满足"基本情况 + 归纳步骤"的结构?例如:空集合大小为 0;每次添加使大小增 1。空字符串解析为空列表;非空字符串解析为头部元素加上剩余部分的解析结果。

**这 5 个问题为什么是完备的提取启发式?** 因为它们覆盖了性质的所有维度:不变性(时间维度)、保证(结果维度)、关系(变换维度)、等价性(比较维度)、结构性(递归维度)。任何一个有意义的属性,都可以被归入至少一个维度。

**在逆向工程中如何使用这 5 个问题?** 不是用来生成随机测试,而是作为**属性发现工具**:对每一个已提取的契约,依次问这 5 个问题,看哪些问题的答案能产出有价值的属性。有些问题的答案可能是"没有有意义的属性",这是正常的 — 目标是系统性地探索,不是为每个维度都找到一个属性。

> **参考文献**:
> - Hughes, J. "How to Specify It!" *Chalmers University of Technology*, 2019.
> - MacIver, D. "In praise of property-based testing." *deadjournal.com*, 2016.

---

## 3. Daikon / Houdini: Generate-Check-Keep Cycle

**Daikon** 由 Michael Ernst 等人在 2001 年发表(TSE),是第一个大规模动态不变量检测工具。它的工作原理分三个阶段:

1. **观察 (Observation)**:在目标程序的关键点(invariants points: 函数入口、出口、循环头)插入探针,记录每次执行的变量值。
2. **候选生成 (Candidate Generation)**:对记录到的值,应用一组预定义的不变量模板(daikon.split, daikon.inv.unary, daikon.inv.binary 等),生成候选不变量。例如,如果观察到变量 `x` 在 100 次执行中始终 `> 0`,则生成候选 `x > 0`。
3. **证伪与淘汰 (Falsification)**:随着更多执行被观察,如果一个候选被某次执行违反,则立即淘汰。最终报告的"不变量"是那些从未被违反的候选 — 即**幸存者(survivors)**。

**Houdini** 由 Cormac Flanagan 和 K. Rustan M. Leino 在 2001 年发表(FME),采用了类似但静态的方法:

1. **候选生成**:为程序生成大量候选注解(annotations),每个都是潜在的不变量或后置条件。
2. **静态验证**:用 ESC/Java 扩展检查验证器对候选注解进行验证。如果验证器能证明某个候选,则保留;如果找到反例,则淘汰。
3. **迭代到不动点**:被淘汰的候选可能使其他候选失效(因为它们互相依赖),所以循环执行验证→淘汰,直到没有新的淘汰发生 — 即达到不动点(fixpoint)。

**两种方法的共同模式是:规格是证伪的幸存者,而非直接提取的产品。** 这一模式的深刻之处在于:

- 它解决了"读代码然后写下你认为它做什么"的主观性问题 — 不同的工程师会写出不同的规格。
- 它提供了一个客观的筛选机制:只有那些**经受住检验**的声明才能进入最终规格。
- 被淘汰的候选同样是信息 — 它们揭示了"看起来像不变量但实际上不是"的模式,是规格的一部分(反面证据)。

**本 skill 的实际适配**: 我们不运行程序,也不使用形式化验证器。我们使用**代码证据(code evidence)**作为证伪机制:

- **逻辑矛盾**:claim 与代码中的显式逻辑相矛盾(如 claim 说 "x > 0",但代码中有 `x = 0` 的路径)。
- **边界路径**:存在某条代码路径使 claim 失效(如异常处理路径绕过了 claim 的保证)。
- **错误路径**:错误处理代码显式违反了 claim(如 panic 路径下不变量可能被破坏)。
- **状态变更**:某段代码修改了 claim 声称不变的字段。

这种方法不如 Daikon 的动态观察全面,也不如 Houdini 的静态验证严谨,但它在纯静态代码分析的约束下,提供了系统化的证伪框架。

> **参考文献**:
> - Ernst, M. et al. "Dynamically Discovering Likely Program Invariants to Support Program Evolution." *IEEE TSE*, 27(2), 2001.
> - Flanagan, C. & Leino, K.R.M. "Houdini, an Annotation Assistant for ESC/Java." *FME 2001*, LNCS 2021.

---

## 4. Abstract State Machines (Gurevich / Börger)

**Abstract State Machines (ASM)** 是 Yuri Gurevich 在 1980 年代提出、后经 Egon Börger 等人系统化的计算模型。其核心思想是:**任何算法都可以被忠实地建模为一个在抽象状态上的状态机,其中状态被表示为一阶逻辑结构(first-order structures)。**

ASM 与普通有限状态机的关键区别在于"抽象"二字。普通状态机的状态是枚举值(如 `IDLE`, `RUNNING`, `STOPPED`),而 ASM 的状态是整个一阶结构 — 包括所有的变量、集合、关系、函数。一个 ASM 的步骤是从一个结构到另一个结构的变换,变换由一组并行执行的更新规则定义。

**逐步抽象 (Stepwise Abstraction)** 是将代码转化为 ASM 的方法论,由 Ferrarotti 等人在 2020 年系统化:

1. **实现级 ASM**:直接从代码提取,每个函数对应一组更新规则,状态包含所有实例变量。这是最细粒度的建模。
2. **逐步抽象**:忽略实现细节(如内部缓存、临时变量、优化结构),只保留对系统外部行为有意义的状态区分。
3. **高层抽象模型**:最终的 ASM 只包含对理解系统行为真正重要的状态和转换。

**为什么状态模型应该是一等工件(First-Class Artifact)?**

在旧的 skill 模型中,状态转换被标记为场景的 Kind (`state-transition`)。这种做法有一个根本问题:状态信息被分散在多个场景中,没有统一的状态空间视图。一个系统有 5 个状态、8 个合法转换、3 个非法转换 — 这些信息散落在 20 个场景的 Given/When/Then 中,没有人能看到完整的状态图。

将状态模型提升为一等工件意味着:

- 有独立的状态枚举、完整的转换表、显式的非法转换列表。
- 跨状态不变量(cross-state invariants)可以独立于任何场景被声明。
- 状态模型的正确性可以独立于场景被验证。

ASM 的理论保证是:**任何系统的行为都可以被忠实地建模为 ASM**,因此提取状态模型不是"一种可能的好做法",而是"完整的系统理解所必需的"。

**(n,m)-抽象概念** (Ferrarotti et al. 2020) 允许在不同粒度级别建模:一个 `(n,m)`-抽象从 n 个实现步骤中抽象出 m 个规范步骤,其中 m << n。这意味着我们不需要建模每一个内部函数调用 — 只需要建模那些改变了系统可观测状态的步骤。

> **参考文献**:
> - Gurevich, Y. "Sequential Abstract State Machines Capture Sequential Algorithms." *ACM TOCL*, 1(1), 2000.
> - Ferrarotti, F. et al. "A New Thesis Concerning Abstract State Machines." 2020.

---

## 5. Characterization Testing (Feathers) — Inflection Points

Michael Feathers 在 2004 年出版的 *Working Effectively with Legacy Code* 中提出了**特征测试 (Characterization Testing)** 的概念:对遗留系统编写测试,目的不是验证正确性,而是**描述当前行为**。他的一个核心洞察是关于**拐点 (Inflection Points / Seams)** 的:

> **"Code changes behind the inflection point cannot affect the system without passing through the inflection point."**

拐点是系统中的边界 — 在这个边界上,内部实现的变化必须通过这个点才能影响外部行为。这意味着:

1. **从拐点开始,向外扩展**:理解了拐点处的行为,就理解了系统对外暴露的全部行为。拐点就像一个"信息瓶颈" — 所有内部变化都必须在此处呈现。
2. **拐点不同于"所有公共 API"**:公共 API 是语法层面的边界(所有 `pub fn`),而拐点是语义层面的边界 — 它关注的是**行为真正发生变化的地方**。一个只有 getter 的 API 不是有意义的拐点,因为它没有行为变化。

**如何识别拐点?** 实践中的拐点包括:

- **公共 API 表面**:所有可以从外部调用的操作。这是最基本的拐点类型。
- **状态变更点**:任何修改持久状态(内存中的可变状态、磁盘上的数据)的操作。状态变更是行为变化的关键来源。
- **I/O 边界**:网络通信、文件读写、系统调用 — 这些是系统与外部世界的边界,也是行为最不可预测的地方。
- **错误处理点**:错误处理和恢复逻辑是行为分歧的关键来源 — 正常路径和错误路径可能产生截然不同的行为。

**拐点对逆向工程的指导意义**: 不需要遍历系统的每一行代码。从拐点出发,识别该拐点"保护"的代码区域,对每个拐点提取契约和属性。如果一个拐点覆盖了 100 行内部代码,这 100 行的行为规格已经通过拐点的契约被捕获了。

拐点之间如果有依赖关系(一个拐点的输出是另一个拐点的输入),则需要考虑它们之间的契约传递 — 但这是拐点级别的组合,而非逐行级别的分析。

> **参考文献**: Feathers, M. *Working Effectively with Legacy Code*. Addison-Wesley, 2004. Chapter 13: "I Need to Make Changes. What Tests Should I Write?"

---

## 6. Reversa (2026) — Confidence and Gaps

**Reversa** 是 2026 年发表的多智能体逆向文档工程框架(arXiv:2605.18684),专门解决"从代码中自动生成可信文档"的问题。它的核心贡献不在于提取技术本身,而在于**提取产出的质量管理**:

1. **操作契约 (Operational Contracts)**:Reversa 要求文档以契约形式表达,而非以叙述形式表达。"系统在处理大文件时使用流式处理"是叙述;"当 input_size > 1MB 时,系统保证内存占用 < 50MB"是操作契约。前者无法验证,后者可以。

2. **可追溯性 (Traceability)**:每一个声明都必须追溯到具体的代码位置。这不是"附加信息",而是声明本身的一部分 — 没有代码引用的声明不被视为完成。

3. **置信度评分 (Confidence Scoring)**:Reversa 为每个声明标注置信度(high/medium/low),明确告知下游消费者哪些声明可以信赖、哪些需要进一步验证。这解决了传统逆向文档的一个痛点:所有声明看起来同等权威,但实际上有些是从代码中明确读出的,有些是推断的。

4. **缺口感知 (Gap Awareness)**:Reversa 把"无法提取的信息"作为一等工件记录,而非简单忽略。一个 gap 文档列出的"不知道"和"不确定"对于下游消费者来说,可能比不完整的确定性声明更有价值 — 因为它明确划定了可信范围的边界。

**本 skill 对 Reversa 思想的采纳体现在多个层面:**

- 契约和属性的每个条款都必须有 `evidence:` 标注(可追溯性)。
- 每个契约都有 `Confidence:` 字段(置信度评分)。
- `gaps.md` 作为一等产出文件存在(缺口感知)。
- `pending_human_input.md` 记录需要人工验证的项目(置信度边界)。
- Phase 4 的证伪循环本质上是 Reversa 的"声明 → 验证 → 分级"流程的实现。

Reversa 本身是一个元框架 — 它定义了"好的逆向文档应该是什么样子",但不提供具体的提取技术。本 skill 的其他方法论(DbC、PBT、ASM、Feathers)提供了 Reversa 需要的实际提取能力。

> **参考文献**: Reversa: Multi-Agent Reverse Documentation Engineering. arXiv:2605.18684, 2026.

---

## Methodology Synthesis

### 共享的反模式:枚举

上述 6 种方法论共享一个反模式:它们都反对**场景枚举 (scenario enumeration)** 作为规格的生成方式。但它们反对的理由各有侧重:

- **DbC**:枚举不能表达通用性 — "输入 x 时返回 y"无法告诉你输入 z 时会怎样。契约可以。
- **PBT**:枚举的声称比它能证明的更强 — 3 个例子的测试实际上声称"对所有输入都正确",但只验证了 3 个。属性声明了真正的声称范围。
- **Daikon/Houdini**:枚举没有证伪机制 — "我认为代码做 X"可能只是因为我没看到它不做 X 的路径。幸存者经过了证伪。
- **ASM**:枚举没有系统性 — 你可能覆盖了 90% 的行为但没有看到关键的状态不一致。状态机提供了完整的状态空间视图。
- **Feathers**:枚举没有聚焦 — 它可能花大量精力在不重要的代码上。拐点告诉你应该关注哪里。
- **Reversa**:枚举没有质量保证 — 它不区分确定和不确定、有据和无据。置信度和缺口提供了质量维度。

### 共享的模式:搜索空间 + 证伪 + 排名

更深一层,这 6 种方法论共享一个元结构:

1. **搜索空间 (Search Space)**:它们定义了一个结构化的空间来放置候选规格。
   - DbC: 前置条件 / 后置条件 / 不变量的空间
   - PBT: 5 种属性类型的空间
   - ASM: 状态 / 转换的空间
   - Feathers: 拐点处的行为空间

2. **证伪机制 (Falsification Mechanism)**:它们定义了如何淘汰不正确的候选。
   - Daikon: 运行轨迹中的反例
   - Houdini: 形式化验证器的反例
   - 本 skill: 代码证据(逻辑矛盾、边界路径、错误路径)

3. **排名机制 (Ranking Mechanism)**:它们定义了如何从幸存者中选择最有价值的。
   - Reversa: 置信度评分
   - DbC: 契约强度(前置条件越弱 + 后置条件越强 = 契约越强)
   - PBT: 属性的普遍性(覆盖的输入空间越大越强)

### 为什么需要组合

每种方法论解决了一部分问题,但都有盲点:

| 方法论 | 解决的问题 | 盲点 |
|--------|----------|------|
| **DbC** | 定义每个操作的规格骨架 | 不能发现"跨操作的全局不变性质" |
| **PBT** | 发现跨操作的性质 | 不知道应该在哪里寻找(需要拐点指导) |
| **Daikon/Houdini** | 消除不正确的声明 | 需要种子候选集(DbC + PBT 提供) |
| **ASM** | 建模状态行为 | 不知道哪些状态值得建模(需要拐点指导) |
| **Feathers** | 告诉你在哪里找 | 不告诉你找什么(需要 DbC + PBT 提供提取框架) |
| **Reversa** | 保证产出质量 | 元框架,需要其他方法提供实际内容 |

因此本 skill 的 Phase 1-6 不是 6 个独立的步骤,而是一个互相喂养的管道:

```
Phase 1 (拐点) → 告诉 Phase 2 在哪里提取契约
Phase 2 (契约) → 告诉 Phase 3 对什么发现属性
Phase 2+3 (契约+属性) → 为 Phase 4 提供候选声明
Phase 4 (证伪) → 精炼 Phase 2+3 的产出
Phase 5 (状态模型) → 独立维度,补充契约+属性无法覆盖的状态行为
Phase 6 (真值锚定) → 对无法静态确定的声明进行动态验证
```

每个 Phase 的输出质量依赖于前序 Phase 的输出质量 — 这比一次性枚举所有可能场景然后筛选的方式,产出了更完整、更准确、更可维护的规格。
