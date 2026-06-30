---
name: harness-bdd-design
description: Cucumber/Gherkin BDD 最佳实践指导技能，负责测试场景的 Discovery（探索）和 Formulation（表述），生成符合Harness工程的测试用例设计，帮助团队编写高质量、可执行、可维护的行为驱动开发规格。
---

# BDD Feature 文件生成技能

## 职责

负责 **Discovery + Formulation** 两个阶段，生成 FT 级行为规格 Feature 文件供后续 Automation 执行。
**不负责** Automation 阶段（Step Definition、World 管理、证据收集等），由 `harness-bdd-coding` skill 负责。

> **FT 级 BDD 定位**：BDD 场景验证**系统通过外部可观察入口的端到端行为**（REST API、CLI、用户界面），不验证单个函数、内部字段、数据库行。
> — "BDD test frameworks are not meant for writing unit tests." — Automation Panda

## 输出约定

- **路径**：上下文缺失路径，应该向用户提问。若用户忽略，则默认写在`.opencode/harness/features/<module-name>-*.feature`
- **禁止**：Feature 文件经人工审核通过后，编码阶段**禁止修改**

## §1 BRIEF 准则（Cucumber 核心）

| 字母 | 含义 | 要求 | 常见反模式 |
|------|------|------|-----------|
| **B** | Business language | 用业务术语，无技术词汇 | 同一词在不同上下文含义不同（如 address、user） |
| **R** | Real data | 用真实具体数据，不用抽象占位 | 依赖特定生产数据（如真实客户 ID "1234"） |
| **I** | Intention revealing | 描述意图，不描述机制 | 使用 UI 术语（如 "click button"、"fill input"） |
| **E** | Essential | 移除不相关的附带细节 | 规则只与日期有关却用日期+时间 |
| **F** | Focused | 一个场景只验证一条业务规则 | 场景依赖的规则没变，场景却失败 |
| **Brief** | 简短 | **大多数场景 ≤ 5 行** | 产品负责人从不读的长场景 |

来源：[Cucumber 官方博客 Keep your scenarios BRIEF](https://cucumber.io/blog/bdd/keep-your-scenarios-brief)

### 步骤数量硬规则

| 范围 | 状态 | 处理 |
|------|------|------|
| 3-5 步 | **理想** | Cucumber Gherkin Reference 官方推荐 |
| 5-7 步 | 可接受 | 复杂 FT 允许 |
| 8-9 步 | 最大上限 | Andy Knight "single-digit step count" 上限 |
| ≥10 步 | **违规** | 必须拆分场景 |

来源：Cucumber Gherkin Reference 原文 *"we recommend 3-5 steps per example"*

### 核心设计自检

> "Will this wording need to change if the implementation does?" — 是 = 太贴近实现，需重写。
> — Cucumber 官方 Writing Better Gherkin

## §2 Given / When / Then 纪律

### 步骤顺序：严格单向，禁止重复

```
Given(0..N) → When(1) → Then(1..N)
```

- **Given**：建立前置状态（precondition），描述系统已有状态。禁止出现用户交互动作。
- **When**：触发**单一**动作或事件（用户请求、系统事件），When应该是真实函数调用，禁用mock。
- **Then**：验证可观察结果（observable outcome）。
- **And / But**：续接**同类型**步骤，不切换到下一类型。

**每个场景只能有 1 个 When-Then 对**。出现多个 When-Then → 按行为拆分为独立 Scenario。

来源：Andy Knight *"A good scenario has only one When-Then pair"*；CucumberStudio；Gherkin Guidelines for AI

### 各步骤类型 And 数量上限

| 步骤 | And 上限 | 说明 |
|------|---------|------|
| Given | 最多 2-3 | 复杂前置条件允许 |
| When | 最多 0-1 | When 应为单一动作 |
| Then | **最多 2** | 例外情况最多 3 |

来源：CucumberStudio *"Decrease And to a reasonable value, say, 2-3 Ands for one Given, When, or Then step"*

## §3 可观察结果原则（Then 的强约束）

> "You should only verify an outcome that is observable for the user (or external system). Changes to a database are usually not."
> "While it might be tempting to implement Then steps to look in the database — **resist that temptation!**"
> — Cucumber 官方 Gherkin Reference, Then 章节

### 什么算"可观察"

| ✅ 可观察（FT 级） | ❌ 不可观察（单元/集成级） |
|------|------|
| HTTP 响应码 + 响应体 | 数据库里的行数 |
| CLI 标准输出/退出码 | 内部 Rust struct 字段值 |
| 用户界面显示的内容 | 单个函数的返回值 |
| 系统发出的消息/日志摘要 | 内存中缓存的状态 |
| 通过 API 可查询的状态 | 私有方法调用是否执行 |

### §3.1 嵌入式库 API 级 BDD 变体（FlashDB 类项目适用）

> 通用 FT 级 BDD 面向 REST/CLI/UI，把"struct 字段、函数返回值"列为不可观察。但**嵌入式库没有 REST/CLI/UI**，它的**公共 API 返回值 + 通过 API 回读的数据 + 通过 API 查询的内部状态**就是唯一的外部可观察面。强行套用通用规则会导致嵌入式库无任何可测入口。

对**无外部进程边界、以库形式被链接**的被测系统（no_std 库、嵌入式 C/Rust 库），按以下变体放宽"可观察"定义：

| 类别 | 通用 FT 级判定 | 嵌入式库变体判定 | 说明 |
|------|---------------|-----------------|------|
| 公共 API 返回码 | ❌ 不可观察 | ✅ 可观察 | 库的唯一出口；断言 `返回值等于 FDB_NO_ERR` 合规 |
| 通过公共 API 回读的数据 | ❌ 不可观察 | ✅ 可观察 | 如 `fdb_kv_get` 读回的值；验证写入-读出一致合规 |
| 通过公共 getter/iter 查询的状态 | ❌ 不可观察 | ✅ 可观察 | 如 `fdb_tsl_query_count` 返回值、迭代器产出的元素序列 |
| 公共 struct 的**公有字段** | ❌ 不可观察 | ✅ 可观察 | 仅当字段为 `pub` 且 API 文档承诺其语义时（如 `init_ok`）；私有字段仍禁止 |
| 公共 struct 的**私有字段** | ❌ 不可观察 | ❌ 仍禁止 | 必须通过公共 API 间接观察 |
| 内部扇区/存储布局 | ❌ 不可观察 | ❌ 仍禁止 | 属实现细节，改实现不应改测试 |
| 调试断言 `FDB_ASSERT` 触发 | — | ✅ 可观察（panic 行为） | 嵌入式库以 panic/abort 表达契约违反；断言 `触发 FDB_ASSERT 断言失败` 合规 |

**判定口诀**：若被测系统是"库"而非"服务"，则"公共 API 调用 + 其返回值/回读数据/公有字段"整体视为一次外部观察，等价于通用 FT 中的"HTTP 请求 + 响应"。仍属实现细节的（存储布局、私有字段、内部函数）继续禁止。

**适用前置条件**（全部满足才启用变体）：
1. 被测系统是库（`lib`/`no_std`），无独立进程/网络入口
2. 被观察的 API/字段在公共 API 文档或头文件中对外承诺
3. 断言不依赖具体的内存布局、字节偏移、存储顺序

### 可观察结果模板（替代原函数级模板）

| FT 类型 | 模板 | 示例 |
|--------|------|------|
| REST API | HTTP status = `<code>` + body contains `"<text>"` | `HTTP status code is "201"`, `body contains "connector-created"` |
| 系统状态查询 | `<resource>` status is `"<state>"` | `connector status is "RUNNING"` |
| 集合计数查询 | `<resource>` has `<N>` `<items>` | `connector has 3 tasks in RUNNING state` |
| CLI 输出 | command exits with `0` and prints `"<text>"` | `command exits with 0 and prints "offset saved"` |
| 错误响应 | response status is `<error_code>` + error field contains `"<msg>"` | `response status is 400`, `error contains "tasks.max is required"` |
| 嵌入式库 API 返回码 | 返回值等于 `<code>` | `返回值等于 FDB_NO_ERR` |
| 嵌入式库 API 回读 | 调用 `<get_api>` 返回 `<value>` | `调用 fdb_kv_get(db, "hostname") 返回字符串 "sensor-01"` |
| 嵌入式库 API 计数 | 调用 `<count_api>` 返回值为 `<N>` | `返回值为 3` |
| 嵌入式库断言契约 | 触发 `<ASSERT>` 断言失败 | `触发 FDB_ASSERT 断言失败` |

### 断言强制检查清单

每条 Then 必须满足，不满足即拒绝生成：

```
□ 描述可观察结果（REST/CLI/UI，或嵌入式库公共 API 返回值/回读数据/公有字段——见 §3.1），非内部实现
□ 期望值是具体字面量（见下方"期望值字面量硬规则"）
□ 不使用模糊词："正确""完整""正常""有效""成功"
□ 不是负面断言："不应""不能""无法""禁止"
□ 每个 Then 验证单一可观察结果（多断言拆分为多个 Then+And）
□ 确定性（多次运行一致，不依赖随机值/排序/外部波动数据）
```

### 期望值字面量硬规则（确定性断言核心）

每条 Then 的期望值**必须是具体字面量**，禁止任何非确定表达：

| ❌ 禁止 | ✅ 允许 | 说明 |
|--------|--------|------|
| 公式 `返回值为 (4096 - HDR) / SIZE` | `返回值为 62` | 公式应预先算出具体整数；若依赖配置则用 Scenario Outline 把配置参数化、Examples 给出各行算好的具体值 |
| 描述性 `value_len 之和` | `iterated_value_bytes 等于 40` | "之和"需在 Given 已给具体数据时由人工算出 |
| 描述性 `每扇区容量` | `返回值为 62` | 同上 |
| 未来时态 `后续写入前会触发 GC` | `gc_request 标志为 true` | Then 必须断言 When 之后**当前**可观察状态，禁止"会/将/后续"等未来时态 |
| 引用外部 `与生产一致` | `返回字符串 "sensor-01"` | 期望值不得依赖外部系统当前状态 |
| 占位无值 `返回正确值` | `返回 64` | 禁止"正确/有效/正常"等无值模糊词 |

**依据**：BDD 的 Then 是可执行断言，期望值必须能被测试代码直接 `assert_eq!(actual, <expected>)`。公式/描述性短语/未来时态都无法直接断言，会把"算期望值"的负担推给 step 实现者，导致断言值漂移、测试意义不明。

**例外**：Scenario Outline 的步骤中可使用 `<placeholder>`，但其期望值必须在该行 Examples 中给出具体字面量。

来源：Gherkin Guidelines for AI *"MUST make Then outcomes observable and checkable"*；*"MUST NOT use vague outcomes like 'it works' without stating how it is known."*

## §4 Scenario Outline + Examples = 确定性断言核心机制

Examples 表格是 FT 级 BDD 实现"同一行为、多组数据、确定性可重现"的标准机制。

### 为什么 Examples = 确定性

1. **步骤中无硬编码数据** — 数据全部来自表格，步骤模板可复用
2. **每行 = 一次确定性测试** — 相同输入必产生相同输出
3. **表格即规格** — 业务方直接读表格理解所有预期行为
4. **边界条件显式化** — 边界用例作为行存在，人人可见

### 决策规则

```
1. 先写一个最能说明业务的普通 Scenario
2. 如果只是"同一行为换几组关键数据" → 提炼为 Scenario Outline + Examples
3. 引入参数后可读性反而下降 → 退回普通 Scenario
```

| 维度 | 用 Scenario Outline + Examples | 不用 |
|------|-------------------------------|------|
| 行为结构 | Given/When/Then 结构完全相同 | 列实际代表不同流程（应拆场景） |
| 数据差异 | 不同等价类、边界值、权限、API 版本 | 同一等价类只是重复数据 |
| 可读性 | 参数化更清晰 | 表格超过 ~8 行且无新行为价值 |
| 列独立性 | 每列独立变化（A 变 B 不必须变） | 多列总是联动（说明应合并为一列） |

来源：Cucumber 官方 Gherkin Reference；Prickles.org *"Every column has to vary independently"*

### Background 使用规则

- **≤ 4 行**，仅放共享的 Given 前置条件
- Background 中的 steps 对 Feature 内所有场景生效
- 仅放"系统级前置"（如 API 版本、集群已启动），不放"测试数据"

来源：Cucumber 官方；CucumberStudio

## §5 反模式修正（FT 级）

| # | 反模式 | 修正 |
|---|--------|------|
| 1 | 函数级断言 `Method::count() == 5` | 改为可观察结果：`resource has 5 items via API query`；嵌入式库改 `调用 <count_api> 返回值为 5`（§3.1） |
| 2 | 一个 Scenario 多个 When-Then 对 | 拆分为独立 Scenario |
| 3 | 命令式细节（点击按钮/填输入框/URL） | 改声明式，描述业务意图 |
| 4 | DB/内部状态验证 `database table has 1 row` | 改为外部可观察：`via API, resource exists`；嵌入式库改通过公共 getter/iter 验证（§3.1） |
| 5 | 模糊结果词 "系统应正常""操作应成功" | 用具体可观察值（status code、state、count） |
| 6 | 硬编码脆弱数据（排序/文案/随机值） | 隐藏细节，描述关系而非具体值 |
| 7 | 硬编码重复验证（3 个同类 Then） | 改为 Scenario Outline + Examples |
| 8 | Scenario Outline 堆同类重复数据 | 仅用于不同等价类/权限/API 版本 |
| 9 | 多个 And 后 Then 描述内部实现 | Then 仅描述 FT 级可观察结果 |
| 10 | 场景依赖执行顺序 | 每个场景自建数据分区，独立可重复 |
| 11 | **Then 期望值非字面量**（公式/描述性短语/"之和"/"每扇区容量"） | 预先算出具体整数/字符串；依赖配置则用 Scenario Outline + Examples 给每行算好的具体值（§3 期望值字面量硬规则） |
| 12 | **Then 未来时态**（"后续…会…""将触发"） | Then 必须断言 When 之后**当前**可观察状态；若行为发生在"下次操作"，改为断言当前可见的标志/状态字段，或拆出独立 Scenario 把触发动作放 When（§3 期望值字面量硬规则） |
| 13 | Scenario Outline Examples 列联动（A 列值由 B 列派生） | 删除派生列，仅保留独立变化的列（§4 列独立性） |

## §6 FT-Level 模板

### 模板 1：REST API 请求 + 响应（最常见于 Kafka Connect）

```gherkin
Feature: Connector lifecycle via REST API

  Scenario: Creating a connector with valid config returns 201 and running state
    Given the Connect cluster is running
    When a POST request is sent to "/connectors" with config:
      | name         | tasks.max | connector.class        |
      | my-source    | 3         | FileStreamSource       |
    Then the HTTP status code should be "201"
    And the response body contains "connector-created"
    And connector "my-source" status is "RUNNING"
```

### 模板 2：Scenario Outline + Examples（同一行为，多组数据）

```gherkin
Feature: Connector config validation

  Scenario Outline: Create connector with invalid <missing_field> should be rejected
    Given the Connect cluster is running
    When a POST request creates connector "<name>" with config "<config>"
    Then the HTTP status code should be "400"
    And the response body contains "<error_message>"

    Examples:
      | missing_field  | name      | config              | error_message          |
      | tasks.max      | source-01 | {}                  | tasks.max is required  |
      | connector.class| source-02 | {"tasks.max":"3"}   | connector.class missing|
      | empty name     | (empty)   | {"tasks.max":"3"}   | name must be non-empty |
```

### 模板 3：多角色权限 FT（ownCloud 模式）

```gherkin
Feature: Connector access control

  Background:
    Given user "alice" has been created with role "admin"
    And user "bob" has been created with role "viewer"

  Scenario Outline: <role> user <expected> delete connector
    Given connector "target" exists
    When user "<user>" sends a DELETE request to "/connectors/target"
    Then the HTTP status code should be "<status>"

    Examples:
      | role   | user  | expected         | status |
      | admin  | alice | should be able to| 204    |
      | viewer | bob   | should not       | 403    |
```

### 模板 4：嵌入式库 API 级（FlashDB 类，适用 §3.1 变体）

> 适用条件：被测系统是库（no_std/嵌入式），无 REST/CLI/UI 入口。公共 API 返回值、回读数据、公有字段、断言契约触发均视为可观察。

```gherkin
Feature: KVDB 键值对写入与读取

  Background:
    Given KVDB 实例已初始化，名称为 "config"，扇区大小为 4096 字节

  Scenario: 写入并读取字符串 KV
    When 调用 fdb_kv_set(db, "hostname", "sensor-01") 写入字符串
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get(db, "hostname") 返回字符串 "sensor-01"
```

```gherkin
Feature: KVDB 配置校验

  Scenario Outline: 写入超长 key 名返回名称错误
    When 调用 fdb_kv_set 写入 key 名长度为 <key_len> 的 KV
    Then 返回值等于 <错误码>

    Examples:
      | key_len | 错误码          |
      | 64      | FDB_NO_ERR      |
      | 65      | FDB_KV_NAME_ERR |
      | 128     | FDB_KV_NAME_ERR |
```

```gherkin
Feature: TSDB 初始化断言契约

  Scenario: 初始化时未提供 get_time 回调触发断言
    Given get_time 参数为 NULL
    When 调用 fdb_tsdb_init(db, "logdb", "part", NULL, 256, NULL)
    Then 触发 FDB_ASSERT 断言失败
```

**嵌入式库模板要点**：
- Then 期望值必须是具体字面量（错误码名、整数、字符串），禁止公式/描述性短语（§3 期望值字面量硬规则）
- 通过公共 API 回读验证写入-读出一致（如 `fdb_kv_get 返回字符串 "..."`），不直接断言存储布局
- 断言契约用 `触发 <ASSERT> 断言失败`，不断言 panic 的具体消息文本（消息属实现细节）
- Scenario Outline 的 Examples 每行必须给出算好的具体期望值，不在 Then 里写公式

## §7 审核清单

### 审核维度

| 维度 | 检查项 |
|------|--------|
| **FT 级粒度** | 通过外部入口（REST/CLI/UI）交互；嵌入式库可走公共 API（§3.1 变体） |
| **BRIEF** | 大多数场景 ≤ 5 步，全部 ≤ 9 步 |
| **单行为** | 每场景恰好 1 个 When-Then 对 |
| **可观察** | 每个 Then 为可观察结果（通用 FT：非 DB/内部状态；嵌入式库：公共 API 返回值/回读/公有字段/断言契约——§3.1） |
| **期望值字面量** | 每个 Then 期望值为具体字面量或 Examples 行内具体值，禁止公式/描述性短语/未来时态（§3 期望值字面量硬规则） |
| **确定性** | 步骤+Examples 保证多次运行结果一致 |
| **业务规则** | 标题体现业务规则，无遗漏异常/边界 |
| **变量规范** | Examples 占位符与步骤 `<placeholder>` 匹配 |
| **Examples 列独立** | 每列独立变化，无派生联动列（§4） |

### 强制拒绝标准

- Then 步骤验证内部状态（DB 行、私有 struct 字段、私有方法调用）——嵌入式库私有字段同样禁止（§3.1）
- Then 期望值非具体字面量：公式、描述性短语（"之和""每扇区容量"）、未来时态（"后续…会…"）、模糊词（"正确值"）——§3 期望值字面量硬规则
- 出现函数级断言（`method() ==`、`::count()`）——嵌入式库例外：公共 API 返回值断言合规（§3.1）
- 口语化无法生成测试代码
- 负面断言无法运行时验证
- 场景超过 9 步未拆分
- Scenario Outline 存在派生联动列（§4 列独立性）

**拒绝记录格式**：
```markdown
### 审核拒绝：{feature 文件名}
- 原因：{具体问题 + 违反规则编号 §x}
- 建议：{修改方案}
```

## 参考案例（GitHub FT 级最佳实践）

| 项目 | 文件 | 亮点 |
|------|------|------|
| [ownCloud](https://github.com/owncloud/core/blob/86611fbc/tests/acceptance/features/apiWebdavOperations/refuseAccess.feature) | WebDAV API 鉴权拒绝访问 | `Scenario Outline` 覆盖 DAV 协议版本 |
| [ownCloud](https://github.com/owncloud/core/blob/86611fbc/tests/acceptance/features/apiShareReshareToRoot2/reShareDisabled.feature) | 共享权限配置切换 | 配置变更作为 `Given` 前置条件 |
| [Nominatim](https://github.com/osm-search/Nominatim/blob/5b70832d/test/bdd/features/api/search/params.feature) | REST API 多格式响应 | `<cname>` 参数化断言逻辑（不仅参数化数据） |
| [Apache Fineract](https://github.com/apache/fineract/blob/0bc90a57/fineract-e2e-tests-runner/src/test/resources/features/LoanChargeback-Part1.feature) | 贷款状态机时间演进 | 时间旅行 + 状态跃迁（FT 级经典） |

## 参考规范（权威来源）

| 来源 | URL |
|------|-----|
| Cucumber Gherkin Reference（官方规范） | <https://cucumber.io/docs/gherkin/reference/> |
| Cucumber Writing Better Gherkin | <https://cucumber.io/docs/bdd/better-gherkin/> |
| BRIEF Framework（Seb Rose，Cucumber 核心成员） | <https://cucumber.io/blog/bdd/keep-your-scenarios-brief> |
| Andy Knight BDD 101 | <https://automationpanda.com/2017/01/30/bdd-101-writing-good-gherkin/> |
| Gherkin Guidelines for AI (2026, 专为 AI 技能设计) | <https://github.com/AutomationPanda/gherkin-guidelines-for-ai> |
| Cucumber 官方反模式 | <https://cucumber.io/docs/guides/anti-patterns/> |
