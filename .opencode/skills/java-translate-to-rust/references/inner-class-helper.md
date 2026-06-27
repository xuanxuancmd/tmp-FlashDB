> **导航**: [SKILL.md](../SKILL.md) | → [接口回调与循环引用](./callback-cyclic.md)

---

# Java 非静态内部类 — 数据/逻辑辅助

内部类**不作为 Listener/Callback 注册**给外部组件，而是做聚合计算、封装辅助逻辑、或聚合外部字段。

## 核心规则

Java 非静态内部类**隐式持有 `this$0` 外部引用**，可在方法中直接访问外部字段。Rust 等价方案：

| 步骤 | 动作 |
|---|---|
| **1. 识别** | 静态分析内部类方法，列出**实际访问**的外部字段 |
| **2. 升级类型** | 被多实例共享的字段升级为 `Arc<共享类型>`：`int/boolean` → `Arc<AtomicI32/AtomicBool>`；不可变数据 → `Arc<T>`；可变容器 → `Arc<RwLock<T>>`；接口类型 → `Arc<dyn Trait>` |
| **3. 独立 struct** | 内部类 → 独立 struct，字段仅包含提取的 Arc 引用 |
| **4. 共享 Arc** | 外部和内部在构造时 `clone()` Arc，各自持有 |
| **5. 严禁** | 不持 `Arc<OuterType>` / `Weak<OuterType>` / `&OuterType` |
| **6. 命名** | `<外部类名><内部类名>`（snake_case），避免 `Inner`/`Impl` 模糊后缀 |

## 简单示例

**Java**：
```java
class Worker {
    int counter = 0;
    String name;

    class CounterHelper {
        void inc() { counter++; }          // 隐式访问外部 counter
        int count() { return counter; }
    }

    CounterHelper helper = new CounterHelper();
    void doWork() { helper.inc(); }
}
```

**Rust**：
```rust
pub struct CounterHelper {
    counter: Arc<AtomicI32>,   // 仅提取 run()/count() 引用的字段
}

impl CounterHelper {
    pub fn inc(&self) { self.counter.fetch_add(1, Ordering::SeqCst); }
    pub fn count(&self) -> i32 { self.counter.load(Ordering::SeqCst) }
}

pub struct Worker {
    counter: Arc<AtomicI32>,   // 与 helper 共享同一份 Arc
    name: String,
    helper: CounterHelper,
}

impl Worker {
    pub fn new(name: String) -> Self {
        let counter = Arc::new(AtomicI32::new(0));
        let helper = CounterHelper { counter: counter.clone() };
        Self { counter, name, helper }
    }
    pub fn do_work(&self) { self.helper.inc(); }
}
```

## 复杂示例（多字段、跨异步边界）

**Java**：
```java
class WorkerSourceTask {
    Producer producer;
    OffsetStorageReader offsetsReader;

    class SubmitHelper {
        CompletableFuture<Void> submitRecords(List<SourceRecord> records) {
            for (SourceRecord r : records) {
                producer.send(r);
                offsetsReader.saveOffset(r);
            }
        }
    }
}
```

**Rust**：
```rust
pub struct SubmitHelper {
    producer: Arc<dyn KafkaProducer>,            // trait object
    offsets_reader: Arc<dyn OffsetStorageReader>, // 仅提取引用的字段
}

impl SubmitHelper {
    pub async fn submit_records(&self, records: &[SourceRecord]) -> Result<(), ConnectError> {
        for r in records {
            self.producer.send(r).await?;
            self.offsets_reader.save_offset(r).await?;
        }
        Ok(())
    }
}

pub struct WorkerSourceTask {
    base_task: Arc<WorkerSourceTaskBase>,
    submit_helper: Arc<SubmitHelper>,   // 共享给异步 task
    // 其他字段...
}
```

## 常见错误

| 错误做法 | 后果 | 正确做法 |
|---|---|---|
| `struct Helper { owner: Arc<Worker> }`（持整个外部 Arc） | 形成循环引用或悬垂 Weak | 仅提取内部类实际访问的字段 |
| 提取**所有**外部字段"以防万一" | 字段过多，Arc 克隆开销大 | 按方法实际访问分析 |
| `int` → `i32`（未升级为 Arc） | 内外各持独立副本，状态不同步 | 共享可变状态必须 `Arc<Atomic*>` |
| `struct Helper<'a> { name: &'a str }` | 借用链复杂，跨异步失败 | 用 `Arc<String>` 持有所有权 |
