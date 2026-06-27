> **导航**: ← [接口回调与循环引用](./callback-cyclic.md) | [SKILL.md](../SKILL.md)

---

### 9.6 泛型 + instanceof检查 (§1 最后一行)

**Connect启动流程中 `instanceof DistributedHerder` 替代方案**：

```rust
pub struct Connect<H: Herder + 'static> {
    herder: Arc<Mutex<H>>,
}

// Herder trait增加default方法替代instanceof检查
pub trait Herder: Send + Sync {
    // ... 其他方法
    
    // 替代 instanceof DistributedHerder 检查
    fn herder_task(&self) -> Option<HerderTaskHandle> { None }
}

// DistributedHerder使用new_arc()模式支持Weak自引用
impl DistributedHerder {
    pub fn new_arc(/* params */) -> Result<Arc<Mutex<Self>>, String> {
        let dh = Arc::new(Mutex::new(Self { /* fields */ }));
        let weak = Arc::downgrade(&dh);
        dh.lock().map_err(|e| e.to_string())?.self_ref = weak;
        Ok(dh)
    }
    
    pub fn start(&self) {
        // 1:1 Kafka的 herderExecutor.submit(this)
        if let Some(self_ref) = self.self_ref.upgrade() {
            tokio::spawn(async move {
                // self_ref 上的异步任务
            });
        }
    }
}
```
