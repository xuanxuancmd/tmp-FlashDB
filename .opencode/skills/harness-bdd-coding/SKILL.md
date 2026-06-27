---
name: harness-bdd-coding
description: BDD测试代码实现指导技能（技术流程），负责 Automation阶段：Feature拷贝、测试代码生成（Cucumber.rs/java等）、信息采集机制、Evidence收集、AI诊断修复闭环。
---

# BDD测试代码实现技能

## 描述

将审核通过的 Feature 文件转化为 FT/E2E 级别的可执行测试代码。本技能聚焦 **Given/When/Then 如何编写出优秀的测试代码**。

- ✅ 本技能负责：Step 代码实现、测试基础设施使用、Evidence收集、诊断修复
- ❌ 本技能不负责：Feature 设计、Gherkin 写法（由 `harness-bdd-design` 负责）

---

## FT/E2E 测试理念

本技能生成的测试覆盖**跨组件的完整业务流程**，不是单个类粒度的单元测试。

- 业务逻辑执行**真实代码**，不 mock 内部实现
- 仅对**系统边界外部依赖**做 stub（外部 REST API、外部数据库等）
- 利用 embedded/stub 组件替代真实基础设施
- 测试验证**端到端业务行为**，不是某个函数的返回值

---

## Given 编写指南

Given 的职责：**建立确定性的前置状态**，让 When 有一个可控的起点。

### 允许的操作

| 操作 | 说明 |
|------|------|
| 初始化共享状态 | 设置 World/Context 的字段 |
| 通过 stub 组件预置数据 | 嵌入式消息队列创建 topic、嵌入式 DB 插入 fixture |
| 配置外部 API 的 mock 响应 | HTTP Mock Server 定义 stub 行为 |
| 使用 Builder/Factory 构造测试数据 | 类型安全、可复用的数据构造 |
| 启动被测组件 | 启动 connector、task 等 |

### 禁止的操作

- ❌ mock 业务逻辑内部模块
- ❌ 跳过 SUT 直接设置内部状态（如 `world.processed = true`）
- ❌ 伪造业务返回值

### 使用基础设施组件打桩

Given 中经常需要搭建测试环境。根据 Feature 语义，加载对应的 `reference/{lang}/infra-*` 文件获取具体实现：

| Feature 涉及的组件 | 加载文件 | Given 中的典型操作 |
|-------------------|----------|-------------------|
| Kafka/消息队列 | `infra-kafka-{lang}.md` | 创建 topic、初始化 producer/consumer |
| 外部 REST API | `infra-http-mock-{lang}.md` | 配置 mock 响应（状态码、JSON body） |
| 数据库 | `infra-database-{lang}.md` | 启动嵌入式 DB、插入 fixture 数据 |
| Redis/缓存 | `infra-redis-{lang}.md` | 启动嵌入式 Redis、预置缓存 |

**示例思路**（语言无关）：
```
Given a Kafka cluster with topic "orders"
  → 加载 infra-kafka-{lang}.md
  → 创建 MockCluster，create_topic("orders", 4, 3)
  → 初始化 producer/consumer 存入 World

Given an external API returns connector status 200
  → 加载 infra-http-mock-{lang}.md
  → 配置 MockServer：GET /connectors/status → 200 + JSON body
  → 将 MockServer URI 注入被测组件

Given a database with existing offset records
  → 加载 infra-database-{lang}.md
  → 启动嵌入式 DB，插入 fixture 数据
  → 将 DB 连接存入 World
```

### Background 处理

Feature 的 Background 部分对所有 Scenario 生效。如果 Background 超过 5 个 Given 步骤，应使用 Factory 模式合并为一个步骤：

```
❌ 冗长 Background（8 个 Given）
✅ 合并：Given a running source connector with default config
   → 内部使用 Factory 一步建立完整前置状态
```

---

## When 编写指南

When 的职责：**触发被测系统的真实业务逻辑**。

### 铁律：When 中只执行真实代码

| 允许 | 禁止 |
|------|------|
| 调用真实 REST API | **任何形式的 mock** |
| 调用真实内部 Service/Connector/Task | 直接修改共享状态跳过 SUT |
| 发送真实消息到嵌入式队列 | 伪造操作结果 |
| 将操作结果保存到共享状态供 Then 使用 | 在 When 中做断言 |

**判断标准**：如果 When 步骤只是设置一个布尔标志（如 `world.processed = true`），说明跳过了整个 SUT，这个测试没有验证任何东西。

### 结果保存模式

When 执行后，将结果保存到共享状态（World/Context），供 Then 断言使用：

```
When the connector is started
  → 调用真实 start_connector() 方法
  → 将返回结果/状态存入 world.last_response

When a record is produced to topic "orders"
  → 调用真实 producer.send()
  → 记录 world.messages_sent += 1
```

**When 中不做断言**——断言是 Then 的职责。

---

## Then 编写指南

Then 的职责：**验证可观察的业务结果**，每个断言对应 Feature 中 Example 的具体期望值。

### 断言三原则

1. **诊断友好**：每个断言附带上下文（期望值、实际值、关键参数），失败时可直接定位原因
2. **验证存在性，非精确匹配**：通过 ID 查询特定记录是否存在，而非假设数据库为空做全量计数
3. **断言可观察输出**：验证 API 响应、DB 数据、消息内容，而非内部计数器或 mock 变量

### 允许的操作

| 操作 | 说明 |
|------|------|
| 断言可观察的业务结果 | API 响应码、返回体 |
| 通过嵌入式 DB 查询验证数据写入 | 直接查 DB 确认记录存在且字段正确 |
| 通过消息消费者验证消息产出 | 从嵌入式 Kafka consumer 读取并断言 |
| 通过 HTTP Mock Server 验证外部调用 | verify() 确认外部 API 被调用 |
| 使用真实代码或 stub 代码辅助断言 | 调用真实的查询方法来获取断言数据 |

### 禁止的操作

- ❌ mock 任何组件
- ❌ 仅检查 mock 调用次数代替业务验证
- ❌ 假设数据库为空做精确计数（`findAll().len() == 1`）
- ❌ 断言内部实现细节（private 字段、内部计数器）

### 使用基础设施组件验证

Then 中经常需要通过 stub 组件查询数据来断言。根据 Feature 语义，加载对应的 `reference/{lang}/infra-*` 文件：

| Feature 涉及的验证 | 加载文件 | Then 中的典型操作 |
|-------------------|----------|-------------------|
| Kafka 消息验证 | `infra-kafka-{lang}.md` | consumer.recv() 断言消息内容和数量 |
| 外部 API 调用验证 | `infra-http-mock-{lang}.md` | mock_server.verify() 确认调用次数 |
| 数据库写入验证 | `infra-database-{lang}.md` | db.find_by_id() 断言记录存在且字段正确 |
| Redis 缓存验证 | `infra-redis-{lang}.md` | redis.get() 断言缓存值 |

**示例思路**（语言无关）：
```
Then the offset for partition 0 should be 5
  → 加载 infra-database-{lang}.md
  → 通过嵌入式 DB 查询 offset 记录
  → assert_eq!(actual_offset, 5, "partition 0 offset 不匹配")

Then the message should be consumed within 10 seconds
  → 加载 infra-kafka-{lang}.md
  → consumer.recv() 带超时
  → 断言消息 topic、key、value

Then the external API should have been called 1 time
  → 加载 infra-http-mock-{lang}.md
  → mock_server.verify() 验证 expect(1) 已满足
```

---

## Step 函数架构

Step 函数是**粘合层（glue）**，不应包含业务逻辑。

```
Feature 文件 (Gherkin 声明式意图)
    ↓ 匹配
Step 函数 (薄层，1-5 行)
    ↓ 委托
Helper / Service (Builder、Factory、Client 封装)
    ↓ 调用
SUT (被测系统真实代码)
```

- **薄 Step**：只做参数传递和方法委托
- **厚 Step（反模式）**：在 Step 内硬编码配置、直接构造对象、散落多步操作

**Helper 层职责**：
- `*Builder`：构造复杂测试数据对象
- `*Factory` / `*Mother`：提供命名标准化测试实体
- `*Client`：封装 REST/消息队列/DB 操作
- World 方法：封装跨步骤的复合操作

---

## 测试数据管理

### Builder Pattern

为复杂测试对象提供链式构造 API，预填合理默认值，允许按需覆盖。避免在 Step 中硬编码 Map/Dict。

### Object Mother / Factory

为常用测试对象提供命名工厂函数。一个 Factory 步骤可替代冗长的 Background（5+ 个 Given 步骤）。

### `build()` vs `create()`

- `build()` → 创建内存对象（单元测试用）
- `create()` → 持久化到 DB/消息队列（FT 测试用，确保 SUT 可查询到数据）

具体实现见对应的 `infra-*` 文件。

---

## 技术实现流程

```
人工审核通过 (harness-bdd-design完成)
  ↓
Step 1: 拷贝Feature文件 → 测试资源目录
  ↓
Step 2: 生成测试代码（加载 cucumber-{lang}.md + world-{lang}.md + 按 Feature 语义加载 infra-*）
  ↓
Step 3: 运行测试验证
  ↓
  ├─通过 → 完成
  └─失败 ↓
Step 4: 信息采集（加载 evidence-collection-{lang}.md + log-isolation-{lang}.md）
  ↓
Step 5: AI诊断修复 → 定位源码 → 修复 → 重跑
```

### Step 1：拷贝 Feature 文件

- **输入**：`.opencode/harness/specs/*.feature`（已审核）
- **输出**：测试资源目录下的 features 文件
- **命名**：`<module-name>-<functionality>.feature`
- ❌ 禁止拷贝后修改 Feature

### Step 2：生成测试代码

**目录组织原则**：
- Step 文件按**业务领域**组织，非按 Given/When/Then 关键字
- 每个 Step 文件混排 Given/When/Then，按领域概念内聚
- 独立的 support 目录存放 Builder、Factory、Helper
- 共享状态定义（World/Context）独立文件

框架语法和配置 → `reference/{lang}/cucumber-{lang}.md` + `reference/{lang}/world-{lang}.md`

### Step 3：运行测试验证

运行命令 → `reference/{lang}/cucumber-{lang}.md`

### Step 4：信息采集（失败时）

**证据类型**：world_state | stack_trace | logs | step_history | assertion_diff

**失败分类**：

| 分类 | 处理 |
|------|------|
| product_bug | 修改业务代码 |
| flaky_test | 添加等待/重试或 quarantine |
| environment_issue | 修复环境配置 |
| test_bug | 修改测试代码 |

**证据存储**：
```
.opencode/harness/evidence/{run-id}/{scenario-id}/manifest.json
```

Evidence 实现 → `reference/{lang}/evidence-collection-{lang}.md` + `reference/{lang}/log-isolation-{lang}.md`

### Step 5：AI诊断修复闭环

读 evidence → 分析 classification → 定位源码 → 修复 → 重跑 → 迭代

---

## 反模式清单

| 反模式 | 问题 | 修复 |
|--------|------|------|
| **Step 函数堆积逻辑** | Step 变成面条代码 | 三层架构：薄 Step → Helper → SUT |
| **跨场景共享状态** | 全局可变状态并行泄漏 | 每个 Scenario 独立 World |
| **过度 mock** | 测试只验证 mock 行为 | FT 级：mock 仅限外部依赖 |
| **When 中 mock** | 跳过真实业务逻辑 | When 铁律：只执行真实代码 |
| **Then 断言内部状态** | 验证 private 字段 | 断言可观察输出 |
| **假设 DB 为空** | 全量计数断言脆弱 | 通过 ID 查询验证存在性 |
| **Feature-coupled steps** | Step 只能用于一个 Feature | 按领域组织，参数化 |
| **冗长 Background** | >5 个 Given 步骤 | Factory 一个步骤建立完整状态 |
| **断言无诊断信息** | 失败时无法定位 | 断言附带上下文描述 |

### AI 生成代码特别审查

| 审查点 | 说明 |
|--------|------|
| **Step 复用** | 检查是否已有匹配 Step，避免歧义重复 |
| **参数化** | 硬编码变体应提取为捕获组参数 |
| **When 真实性** | 确认 When 调用真实 SUT 而非直接设置共享状态 |
| **Then 可观察性** | 确认 Then 断言可观察结果而非 mock 返回值 |
| **Builder 使用** | 复杂对象应通过 Builder 构造，不应硬编码 Map/Dict |

---

## 禁止事项

1. ❌ **修改 Feature 文件**
2. ❌ **When 步骤中 mock 业务逻辑**
3. ❌ **Then 步骤中 mock 任何组件**
4. ❌ **Placeholder / TODO 代码**
5. ❌ **Evidence 非结构化**
6. ❌ **Step 中打桩内部业务模块**

---

## 与 harness-bdd-design 协作

```
harness-bdd-design → Feature生成 → 人工审核 → 通过 → harness-bdd-coding
                                                         ↓
                                          拷贝 → 生成 → 验证 → 诊断
```

| 阶段 | design | coding |
|------|--------|--------|
| Discovery/Formulation | ✅ | ❌ |
| Feature生成 | ✅ | ❌ |
| 拷贝Feature | ❌ | ✅ |
| 生成测试代码 | ❌ | ✅ |
| 运行验证 | ❌ | ✅ |
| 失败诊断 | ❌ | ✅ |

---

## Reference 文件索引

### Rust (`reference/rust/`)

| 文件名 | 用途 | 加载时机 |
|--------|------|----------|
| `cucumber-rust.md` | Runner配置、Step语法、Hooks | Step 2/3 |
| `world-rust.md` | World定义、状态保存 | Step 2 |
| `infra-kafka-rust.md` | MockCluster、Producer/Consumer | Given/Then 涉及 Kafka |
| `infra-http-mock-rust.md` | wiremock Mock Server | Given/Then 涉及外部 REST API |
| `infra-database-rust.md` | 嵌入式DB、数据验证 | Given/Then 涉及数据库 |
| `infra-redis-rust.md` | 嵌入式Redis | Given/Then 涉及 Redis |
| `log-isolation-rust.md` | 日志采集方案 | Step 4 |
| `evidence-collection-rust.md` | Evidence Writer实现 | Step 4/5 |

### Java (`reference/java/`)

（待创建）
