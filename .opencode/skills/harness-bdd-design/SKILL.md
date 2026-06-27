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

### 可观察结果模板（替代原函数级模板）

| FT 类型 | 模板 | 示例 |
|--------|------|------|
| REST API | HTTP status = `<code>` + body contains `"<text>"` | `HTTP status code is "201"`, `body contains "connector-created"` |
| 系统状态查询 | `<resource>` status is `"<state>"` | `connector status is "RUNNING"` |
| 集合计数查询 | `<resource>` has `<N>` `<items>` | `connector has 3 tasks in RUNNING state` |
| CLI 输出 | command exits with `0` and prints `"<text>"` | `command exits with 0 and prints "offset saved"` |
| 错误响应 | response status is `<error_code>` + error field contains `"<msg>"` | `response status is 400`, `error contains "tasks.max is required"` |

### 断言强制检查清单

每条 Then 必须满足，不满足即拒绝生成：

```
□ 描述可观察结果（来自 REST/CLI/UI），非内部实现
□ 不使用模糊词："正确""完整""正常""有效""成功"
□ 不是负面断言："不应""不能""无法""禁止"
□ 每个 Then 验证单一可观察结果（多断言拆分为多个 Then+And）
□ 确定性（多次运行一致，不依赖随机值/排序/外部波动数据）
```

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
| 1 | 函数级断言 `Method::count() == 5` | 改为可观察结果：`resource has 5 items via API query` |
| 2 | 一个 Scenario 多个 When-Then 对 | 拆分为独立 Scenario |
| 3 | 命令式细节（点击按钮/填输入框/URL） | 改声明式，描述业务意图 |
| 4 | DB/内部状态验证 `database table has 1 row` | 改为外部可观察：`via API, resource exists` |
| 5 | 模糊结果词 "系统应正常""操作应成功" | 用具体可观察值（status code、state、count） |
| 6 | 硬编码脆弱数据（排序/文案/随机值） | 隐藏细节，描述关系而非具体值 |
| 7 | 硬编码重复验证（3 个同类 Then） | 改为 Scenario Outline + Examples |
| 8 | Scenario Outline 堆同类重复数据 | 仅用于不同等价类/权限/API 版本 |
| 9 | 多个 And 后 Then 描述内部实现 | Then 仅描述 FT 级可观察结果 |
| 10 | 场景依赖执行顺序 | 每个场景自建数据分区，独立可重复 |

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

## §7 审核清单

### 审核维度

| 维度 | 检查项 |
|------|--------|
| **FT 级粒度** | 通过外部入口（REST/CLI/UI）交互，未直接调用内部函数 |
| **BRIEF** | 大多数场景 ≤ 5 步，全部 ≤ 9 步 |
| **单行为** | 每场景恰好 1 个 When-Then 对 |
| **可观察** | 每个 Then 为可观察结果（非 DB/内部状态） |
| **确定性** | 步骤+Examples 保证多次运行结果一致 |
| **业务规则** | 标题体现业务规则，无遗漏异常/边界 |
| **变量规范** | Examples 占位符与步骤 `<placeholder>` 匹配 |

### 强制拒绝标准

- Then 步骤验证内部状态（DB 行、struct 字段、私有方法调用）
- 出现函数级断言（`method() ==`、`::count()`）
- 口语化无法生成测试代码
- 负面断言无法运行时验证
- 场景超过 9 步未拆分

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
