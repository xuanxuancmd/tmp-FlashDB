> **导航**: 返回 [SKILL.md](../SKILL.md) | 下一章: [抽象类继承: Trait + Getter](./inheritance-trait-getter.md)

---

## 1. 继承体系翻译方案

Java的继承体系（extends、implements、abstract class）在Rust中需按具体结构特征选择翻译方案。本节按三个维度展开：

| Java继承特征 | Rust方案 | 核心判断依据 |
|-------------|----------|------------|
| 子类数量确定、集中同目录 | Enum + match | 封闭类型体系，无需扩展 |
| 抽象类仅有行为、无状态字段 | Trait（可含默认方法） | 纯行为抽象，无protected字段访问 |
| 多级继承、父类有状态字段 | 逐层嵌套struct + 委托 | 需访问父类字段，"has-a"关系 |

### 1.1 封闭类型体系 → Enum替代

**核心规则**：Java中**子类数量确定、集中同目录、不再扩展**的类型体系，统一用Enum替代继承。

Enum模式的优势：
- 模式匹配穷尽所有分支，编译期保证类型安全
- 避免trait对象开销（无vtable）
- 代码集中在一个文件，维护成本低

#### 适用场景

| Java模式 | 典型示例 | Enum变体设计 |
|----------|---------|-------------|
| 异常层级（同目录） | 多个异常类 → `enum AppError` | 每个异常类 → 一个variant，附带错误信息 |
| 状态/类型枚举 | 抽象状态类 → `enum Status { TypeA, TypeB }` | 每个状态类 → 一个variant，附带状态数据 |

#### 翻译模式

Java多异常类（同目录、固定数量）→ Rust统一enum：
```rust
// Java: DataException, ConfigException, ... → 统一enum
pub enum ConnectError {
    Data { msg: String, cause: Option<Box<dyn std::error::Error>> },
    Config { msg: String, cause: Option<Box<dyn std::error::Error>> },
    Timeout { msg: String },
}
```

#### 决策依据：何时用Enum vs Trait

| 判断条件 | Enum | Trait |
|---------|------|-------|
| 子类型数量是否确定？ | ✅ 固定 | ❌ 可扩展 |
| 是否集中同目录？ | ✅ 同目录 | ❌ 分散 |
| 是否需要其他模块扩展？ | ❌ 不需要 | ✅ 需要 |
| 是否主要用于分类/状态？ | ✅ 是 | ❌ 否 |

> **注意**：若Java子类分散在不同目录，或子类数量不确定，必须用trait + impl方案。此类场景的核心规则：trait必须添加`: Send + Sync`以支持跨线程传递。
