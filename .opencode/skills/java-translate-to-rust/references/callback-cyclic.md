## 1. 接口回调与循环引用

Java中常见的两种对象关系模式在Rust中会暴露所有权约束问题，需要引入适配层解决：

| Java模式 | Rust中的问题 | 典型Java代码 |
|---------|------------|-------------|
| **监听器/观察者模式** | 双向`Arc`引用导致循环依赖 | `store.setListener(this)` |
| **接口回调** + `this`传递 | `Arc<T>` vs `Arc<dyn Trait>`类型不兼容 | `callback.invoke(this)` |
| **跨模块接口实现** | Rust孤儿规则阻止直接`impl` | `class A(in B) implements Trait(in A)` |
| **同语义异签名** | 两个trait功能相同但签名不同 | `onComplete(v)` vs `complete(v)` |

**通用解决方案：适配器模式（Adapter Pattern）**

Rust官方依据（The Rust Book Chapter 10）：
> "当你想让一个类型实现某个trait，但该类型定义在另一个crate中，你不能直接实现。此时可以使用`Wrapper`类型：创建一个新类型包装原始类型，然后为新类型实现目标trait。"

**核心思想**：创建一个薄包装struct，实现目标trait并转发调用到内部的具体类型。

### 1.1 监听器模式的循环依赖

**Java模式**：`class A implements Listener`，且`A`持有`B`（存储了listener），形成双向引用。Java GC可以处理，Rust中`Arc<A>` ↔ `Arc<Listener>`形成所有权循环。

**典型Java代码**：
```java
// Java
class DistributedHerder implements ConfigBackingStoreUpdateListener {
    private ConfigBackingStore configBackingStore;

    public void registerConfigListener() {
        configBackingStore.setListener(this);  // 形成循环引用
    }
}
```

**Rust问题**：
```rust
// ❌ 循环依赖：DH需要Arc<Store>，Store需要Arc<DH>
pub struct ConfigBackingStore {
    listener: Option<Arc<dyn ConfigBackingStoreUpdateListener>>,
}

pub struct DistributedHerder {
    config_backing_store: Arc<ConfigBackingStore>,
    // 如果想让DH实现listener，需要: listener: Arc<DistributedHerder>（循环！）
}
```

**解决方案：Adapter包装**

```rust
// distributed_herder.rs:712 - Adapter定义
pub struct DistributedHerderConfigListenerAdapter {
    dh: Arc<DistributedHerder>,  // 包装具体类型（非Arc<dyn Herder>）
}

// distributed_herder.rs:727 - 实现目标trait
impl ConfigBackingStoreUpdateListener for DistributedHerderConfigListenerAdapter {
    fn on_connector_config_update(&self, connector: &str) {
        self.dh.handle_connector_config_update(connector);  // 转发调用
    }

    fn on_connector_config_remove(&self, connector: &str) {
        self.dh.handle_connector_config_remove(connector);
    }

    fn on_task_config_update(&self, tasks: &[ConnectorTaskId]) {
        self.dh.handle_task_config_update(tasks);
    }

    fn on_connector_target_state_change(&self, connector: &str, target_state: TargetState) {
        self.dh.handle_connector_target_state_change(connector, target_state);
    }
}

// 使用：注册listener
impl DistributedHerder {
    pub fn register_config_listener(self: &Arc<Self>) {
        let adapter = Arc::new(DistributedHerderConfigListenerAdapter {
            dh: Arc::clone(self),  // 包装自己
        });
        self.config_backing_store.set_update_listener(adapter);
    }
}
```

**为什么直接实现不行**：
1. **循环依赖**：`DistributedHerder`直接实现`ConfigBackingStoreUpdateListener`会导致`DH持有Arc<Store>` + `Store持有Arc<DH>`形成循环
2. **Arc<dyn Trait>约束**：`ConfigBackingStore`需要`Arc<dyn Listener>`，但`DistributedHerder`不是trait，无法使用`Arc<dyn DistributedHerder>`
3. **Adapter解决方案**：包装`Arc<DistributedHerder>`（具体类型），Adapter实现`Listener` trait，打破循环（Adapter → DH → Store → Arc<Adapter>不构成循环）

### 1.2 接口回调的trait签名不匹配

**Java模式**：两个接口有相同语义但签名不同的方法（如`onComplete(v)` vs `complete(v)`），Java中可以灵活调用。Rust中trait签名严格，需要适配。

**Callback Adapter（签名适配）**：

```rust
// connect_standalone.rs:348
pub struct HerderCallbackAdapter {
    future_callback: Arc<FutureCallback<ValidateResponse>>,
}

// FutureCallback有complete()/error()方法
// Callback trait有on_completion()/on_error()方法
// Adapter做签名转换
impl Callback<ValidateResponse> for HerderCallbackAdapter {
    fn on_completion(&self, result: ValidateResponse) {
        self.future_callback.complete(result);  // 转发到实际接口
    }

    fn on_error(&self, error: String) {
        self.future_callback.error(error);  // 转发
    }
}
```

### 1.3 Async回调中的Channel适配

**Java模式**：监听器方法接收`this`引用并调用对象方法。Java无所有权问题。

**Rust问题**：`on_rebalance`是async方法，直接调用herder的async方法会导致borrow checker报错（异步回调中持有`Arc`，同时在async上下文中借用）。

**解决方案：Channel Adapter（解耦async约束）**

```rust
pub struct RebalanceListener {
    assignment_sender: mpsc::UnboundedSender<RebalanceMessage>,
}

impl DistributedRebalanceListener for RebalanceListener {
    async fn on_rebalance(&self, assignment: Assignment) {
        // 通过channel转发消息，避免直接调用herder的async方法
        self.assignment_sender
            .send(RebalanceMessage::Rebalance(assignment))
            .await;
    }
}

async fn herder_main_loop(receiver: &mut mpsc::UnboundedReceiver<RebalanceMessage>) {
    while let Some(msg) = receiver.recv().await {
        match msg {
            RebalanceMessage::Rebalance(assignment) => handle_rebalance(assignment).await,
        }
    }
}
```

**Channel Adapter的优势**：
- 解耦监听回调和业务处理
- 避免异步回调中的borrow checker问题
- 监听器不再持有herder的`Arc`，彻底打破循环依赖
- 便于测试（可以注入mock sender）

> **注意**：适配器模式仅用于解决trait约束冲突或循环引用问题，不宜滥用。每多一层adapter增加一层间接调用的复杂度。
