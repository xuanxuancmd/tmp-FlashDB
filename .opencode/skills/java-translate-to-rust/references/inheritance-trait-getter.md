> **导航**: [SKILL.md](../SKILL.md) | → [实体子类继承](./inheritance-concrete-composition.md)

---

# 抽象类继承 → Trait + Getter

Java 抽象类统一映射为 Rust **trait + getter**：

- **trait** 定义抽象类所有方法（`abstract` 方法 + 非 `abstract` 方法）
- **getter** 暴露抽象类成员变量（如有），供 trait default impl 访问
- **具体子类** flat 地 impl 所有 trait，并通过 getter 返回自身 state struct

## 核心原则

| Java 概念 | Rust 映射 |
|---|---|
| `abstract class A` | `trait A` |
| 抽象类成员变量（protected） | 内部 state struct，如 `AState { ... }` |
| **`abstract` 方法** | **trait 方法签名（无 body）** |
| **非 `abstract` 方法（具体方法）** | **trait 方法带 default body**，访问字段时通过 getter |
| `abstract class B extends A` | `trait B: A`（supertrait 约束） |
| `class C extends B`（具体子类） | `struct C { ... }` flat impl A + B 所有方法 |

**关键事实**：
- Rust trait 不继承字段，只继承方法签名 + default body
- Java "抽象类的具体方法" → Rust trait 的 default method
- 具体子类 impl trait 时，只需为 `abstract` 方法提供实现，default 方法可继承/覆盖

## 场景 1：无状态抽象类（无成员变量）

抽象类只定义方法签名，不持有任何状态。所有方法（无论 abstract）都放入 trait。

```rust
pub trait Connector: Versioned {
    // abstract 方法 → trait 方法签名
    #[async_trait]
    async fn start(&mut self, props: HashMap<String, String>);
    async fn stop(&mut self);
    fn task_class(&self) -> &'static str;

    // 非 abstract 方法（Java 中是 public String toString() 这样的具体方法）
    fn is_sink_connector(&self) -> bool { false }  // default impl
}

// 子 trait: 继承 + 新增方法
#[async_trait]
pub trait SinkConnector: Connector {
    // abstract 方法（Java 中是 abstract）
    fn alter_offsets(
        &self,
        connector_config: HashMap<String, String>,
        offsets: HashMap<TopicPartition, Option<i64>>,
    ) -> Result<bool, ConnectError>;

    // 非 abstract 方法（Java 中是 public boolean validate() { ... 有 body } ）
    fn validate_connector(&self, config: &Config) -> ConfigValue {
        config.validate()  // default impl
    }
}

// 具体子类 flat impl 所有 trait
impl Versioned for VerifiableSinkConnector { ... }
impl Connector for VerifiableSinkConnector {
    async fn start(&mut self, props: HashMap<String, String>) { /* 必须实现 */ }
    async fn stop(&mut self) { /* 必须实现 */ }
    fn task_class(&self) -> &'static str { "VerifiableSinkTask" }
    // is_sink_connector() 继承 default
}
impl SinkConnector for VerifiableSinkConnector {
    fn alter_offsets(...) -> Result<bool, ConnectError> { /* 必须实现 */ }
    // validate_connector() 继承 default
}
```

## 场景 2：有状态抽象类（有成员变量）

抽象类的 protected 字段抽象为 state struct，trait 暴露 getter。具体方法可通过 getter 访问 state。

```rust
// Java: abstract class AbstractHerder {
//     protected int generation;
//     protected String clusterId;
//     public abstract void start();          // abstract
//     public boolean isLeader() {            // 非 abstract，访问 generation
//         return generation > 0;
//     }
// }

// === state struct ===
pub struct AbstractHerderState {
    pub generation: i32,
    pub cluster_id: String,
}

// === trait：abstract + 非 abstract 都包含 ===
pub trait AbstractHerder {
    // abstract 方法 → 签名
    async fn start(&mut self);

    // getter → 让 default impl 能访问 state
    fn state(&self) -> &AbstractHerderState;

    // 非 abstract 方法 → default body 通过 getter 访问 state
    fn is_leader(&self) -> bool {
        self.state().generation > 0
    }
}

// === 具体子类 ===
pub struct DistributedHerder {
    state: AbstractHerderState,
    // 20+ 其他字段
}

impl AbstractHerder for DistributedHerder {
    async fn start(&mut self) { /* 必须实现 */ }
    fn state(&self) -> &AbstractHerderState { &self.state }  // 必须实现 getter
    // is_leader() 继承 default，无需 impl
}
```

**要点**：
- state struct 命名 `<抽象类名>State`
- getter 命名统一为 `fn state(&self) -> &XxxState`；多 state 时用具体名
- 子类持有 state struct 作为字段，impl getter 返回引用
- 子类**不应直接访问** `self.state.generation`，应通过 trait 定义具体方法封装

## 多级抽象链

递归应用上述规则：`trait C: B: A`。具体子类 flat 地分别 impl 每个 trait：

```rust
impl A for X { ... }       // 必须实现的 abstract 方法
impl B for X { ... }
impl C for X { ... }
```

**不共享实现，无自动方法转发**——每个 trait 的 abstract 方法独立手写 impl 块。default 方法在各 trait 内独立定义，子类可选择 override。

## 决策检查清单

翻译前快速判断：

- [ ] **父类有状态字段**（protected）？YES → 场景 2（state struct + getter）；NO → 场景 1
- [ ] **子类也是抽象类**（即 trait 链）？YES → `trait B: A` supertrait 约束
- [ ] **子类数量固定、同目录**？YES → 考虑 Enum（详见 [inheritance-enum-trait.md](./inheritance-enum-trait.md)）
- [ ] **是否需多态传递**（Java 中 `(Parent) child` 形式）？YES → **应使用实体子类继承**，返回 [inheritance-concrete-composition.md](./inheritance-concrete-composition.md)

## 反模式

```rust
// ❌ 把具体方法放到 struct impl 块（应放 trait default）
impl DistributedHerder {
    pub fn is_leader(&self) -> bool { self.state.generation > 0 }
}
// 应改为 trait AbstractHerder 的 default method，否则破坏 trait 多态

// ❌ 合并多个抽象层级到同一 trait
pub trait Connector {
    fn start(&mut self);
    fn alter_offsets(&self);  // 这是 SinkConnector 的方法
}
// 违反 1:1 翻译，丢失 Java 层级结构

// ❌ 用结构体包装替代 trait 层级
pub struct SinkConnectorBase { connector: Box<dyn Connector> }
```
