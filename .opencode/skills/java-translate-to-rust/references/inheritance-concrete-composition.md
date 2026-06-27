# 实体子类继承 → 组合 + Trait

Java 实体子类继承（父类有成员变量 / protected 字段被子类访问）→ Rust **组合模式**：子 struct 持有父 struct 作为字段，通过委托访问父方法。

同时，根据"父类是否需要作为多态传递类型"，抽取不同类型的 trait。

## 核心原则

| Java 概念 | Rust 映射 |
|---|---|
| `class B extends A { int y; }` | `struct B { base: A, y: i32 }` |
| `super.f()` / `this.f()` | `self.base.f()`（直接委托） |
| `(Parent) child`（多态 cast） | 共享 trait + `dyn SharedTrait` |
| `class C extends B extends A` | `struct C { base: B }` 其中 `B` 也有 `base: A`（递归） |
| 父类 `abstract` （典型场景：模版类） | 参考 [inheritance-trait-getter.md](inheritance-trait-getter.md) |

## 场景 A：纯组合（无多态传递需求）

子 struct 持有父 struct，委托访问父方法。**不需要抽取共享 trait**。

**判断标准**：Java 中是否存在 `Parent p = new Child()` 的多态 cast？若无，则用场景 A。

```
Java                              Rust
────────                          ────
class A { int x; void f() }       struct A { x: i32 } impl A { fn f(&self) }
class B extends A { int y; }      struct B { base: A, y: i32 }
B.f() → super.f() or this.f()     impl B { fn f(&self) { self.base.f() } }
```

**要点**：
- 子类方法全部转发到 `self.base.xxx()`
- **无 trait**：Java 中不需要多态

## 场景 B：组合 + 多态传递（共享 trait，父子都 impl）

父类需要作为多态传递类型时，**抽取共享 trait，父和所有子类都 impl**。调用处用 `dyn SharedTrait`。

**判断标准**：Java 中是否存在 `Parent p = new Child()` 并通过 `p` 调用方法？或者，方法传参通过父类接收子类，若存在，则用场景 B。

### 案例：MirrorHerder extends DistributedHerder

```rust
pub trait Herder: Send + Sync {
    async fn start(&mut self);
}

impl Herder for DistributedHerder {
    async fn start(&mut self) {
        // ...
    }
}
pub struct MirrorHerder {
    base: Arc<Mutex<DistributedHerder>>,  // 组合父类（Arc 共享）
}

impl Herder for MirrorHerder {
    async fn start(&mut self) {
        self.base.start().await;
    }
}

// 调用处可用 dyn Herder 多态
let herder: Arc<dyn Herder> = Arc::new(mirror_herder);
herder.start().await;
```
## Arc 持有 vs 直接持有

| 持有方式 | 语法 | 何时使用 |
|---|---|---|
| 直接 | `base: Parent` | 子类独占父类，无共享访问 |
| Arc | `base: Arc<Parent>` | 父类需被多处引用（如回调闭包捕获） |
| Arc + Mutex | `base: Arc<Mutex<Parent>>` | 父类内部可变 + 多处引用 |

**案例对应**：
- `ConnectorStatus` → 直接持有
- `WorkerSourceTask` → Arc 持有（回调需捕获 base_task）
- `MirrorHerder` → Arc<Mutex> 持有（父类可变 + 多处引用 + 回调）

## 反模式

```rust
// ❌ 用 trait 替代组合（父类有状态字段时）
pub trait AbstractStatus {
    fn id(&self) -> &str;
    fn state(&self) -> State;
}
// 原因：状态字段不能放在 trait 里，强制用 trait 导致字段重复

// ❌ 组合模式下暴露 pub base
pub struct ConnectorStatus { pub base: AbstractStatus<String> }
// 应该定义委托方法，不要暴露 base，否则破坏封装和 1:1 映射

// ❌ 过度使用 Arc<Mutex>
pub struct Child { base: Arc<Mutex<Parent>> }  // 若 Parent 无需共享，过度设计
// 直接持有 pub struct Child { base: Parent } 即可
```

### 多级的 trait 处理

如果多级继承链中某层需要多态传递（场景 B），为**顶层或所需层**定义共享 trait，所有子类 flat 地 impl：

```rust
pub trait Connector { ... }  // 顶层 trait

// Level 2/3 struct 都 impl Connector（flat 实现，不共享方法）
impl Connector for AbstractConnector { /* delegate to base */ }
impl Connector for FileStreamSourceConnector { /* delegate to base */ }
```

## 委托方法命名约定

| 方式 | 适用场景 | 示例 |
|---|---|---|
| `self.base.method()` | 直接转发 | `pub fn id(&self) -> &str { self.base.id() }` |
| `self.base.method().await` | 异步委托 | `self.distributed_herder.lock().await.start_herder().await` |
| 委托 + 扩展 | 转发后追加逻辑 | `delegate then extend` |
| 扩展 + 委托 | 先执行子类逻辑，再调父类 | `self.init_producer(); self.base.start_task()` |
