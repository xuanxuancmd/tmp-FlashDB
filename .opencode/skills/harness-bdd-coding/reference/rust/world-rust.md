# Rust World State Pattern

本文档详细说明cucumber-rs中World模式的实现机制、状态保存策略以及在Harness工程中的应用。

---

## World模式本质

### 定义

**World**是BDD测试框架中的核心状态管理抽象：

- 每个场景一个全新World实例（隔离性）
- 同场景内所有Step共享`&mut World`（跨步共享）
- 框架自动管理生命周期（自动化）

### cucumber-rs实现

```rust
// cucumber/src/lib.rs (World trait定义)
pub trait World: Sized + 'static {
    type Error: std::fmt::Display;
    
    /// 创建新World实例（每个场景调用一次）
    fn new() -> impl Future<Output = Result<Self, Self::Error>>;
}
```

### 你的项目已有实现

```rust
// connect-runtime/tests/bdd/world.rs (已存在)
#[derive(Debug, cucumber::World)]
pub struct ConnectRuntimeWorld {
    // Task lifecycle
    pub task_id: i32,
    pub task_started: bool,
    pub task_failed: bool,
    
    // Offset management
    pub committed_seqno: u64,
    pub backing_store_configured: bool,
    
    // Config validation
    pub validation_success: bool,
    pub missing_field: String,
    
    // Error handling
    pub tolerance_level: String,
    pub dlq_topic_name: String,
    
    // 日志buffer（建议添加）
    #[serde(skip)]
    pub logs: Option<Arc<Mutex<Vec<u8>>>>,
    
    #[serde(skip)]
    pub subscriber_guard: Option<tracing::subscriber::DefaultGuard>,
}
```

---

## 失败状态保存机制

### cucumber-rs内置机制

**关键发现**：cucumber-rs已内置失败时World状态捕获。

```rust
// cucumber/src/event.rs (内部实现)
pub enum Step<World> {
    Failed(
        Option<regex::CaptureLocations>,  // 正则匹配参数
        Option<Location>,                  // Step定义位置
        Option<Arc<World>>,                // ← 失败瞬间的World快照
        StepError,                         // 错误类型
    ),
}
```

### 捕获时机

1. **Step执行失败**：panic时World moved into `ExecutionFailure::StepPanicked`
2. **Before hook失败**：Hook失败时World captured
3. **After hook访问**：即使失败后，after hook仍能访问`Option<&mut World>`

### 关键特性

- **panic拦截**：cucumber-rs用`catch_unwind()`捕获panic不崩溃
- **Arc<World>封装**：失败时立即保存World到Arc（线程安全）
- **事后访问**：after hook即使失败后也能访问World

---

## 输出World状态的配置

### 方式1：verbose模式（简单）

```bash
# 运行时启用verbose输出World
cargo test --test cucumber -- --verbose
```

### 方式2：自定义Writer（推荐）

```rust
use cucumber::Writer;
use serde_json::json;

pub struct DiagnosticWriter {
    failures: Vec<FailureRecord>,
}

impl Writer for DiagnosticWriter {
    fn handle_step_failed(&mut self, ev: StepFailed) {
        // 提取World状态
        let world_state = ev.world.clone();  // Arc<World>
        
        // 序列化到JSON
        let world_json = serde_json::to_value(&*world_state).unwrap();
        
        let failure = FailureRecord {
            scenario: ev.scenario_name,
            step: ev.step_name,
            location: ev.location,
            error: ev.error.to_string(),
            world_state: world_json,
        };
        
        self.failures.push(failure);
    }
    
    fn finish(&mut self) {
        // 输出诊断报告
        let report = json!({
            "schema_version": "kafka-connect-harness-1.0",
            "failures": self.failures,
        });
        
        std::fs::write("evidence/diagnostic_report.json", 
                       serde_json::to_string(&report).unwrap());
    }
}

// 注册Writer
ConnectRuntimeWorld::cucumber()
    .with_writer(DiagnosticWriter::new())  // ← 添加
    .run_and_exit("./tests/resources/features")
    .await;
```

---

## World设计最佳实践

### 1. 必须派生Debug

```rust
#[derive(Debug, cucumber::World)]  // ← Debug是必需的
pub struct MyWorld {
    pub field1: String,
    pub field2: i32,
}
```

**原因**：verbose输出和失败诊断需要Debug格式化。

### 2. 可选派生Serialize（JSON输出）

```rust
#[derive(Debug, Serialize, cucumber::World)]
pub struct MyWorld {
    #[serde(skip)]  // ← 跳过大字段避免bloat
    large_data: Vec<u8>,
    
    pub important_field: String,  // ← 关键诊断字段
}
```

### 3. 分离测试基础设施和业务状态

```rust
#[derive(Debug, Serialize, cucumber::World)]
pub struct ConnectRuntimeWorld {
    // 业务状态（诊断用）
    pub task_id: i32,
    pub committed_seqno: u64,
    pub validation_success: bool,
    
    // 测试基础设施（输出时跳过）
    #[serde(skip)]
    pub logs: Option<Arc<Mutex<Vec<u8>>>>,
    
    #[serde(skip)]
    pub subscriber_guard: Option<tracing::subscriber::DefaultGuard>,
    
    #[serde(skip)]
    pub temp_dir: Option<std::path::PathBuf>,
}
```

### 4. Default实现

```rust
impl Default for ConnectRuntimeWorld {
    fn default() -> Self {
        Self {
            task_id: 0,
            committed_seqno: 0,
            validation_success: false,
            logs: None,
            subscriber_guard: None,
            temp_dir: None,
        }
    }
}
```

---

## World生命周期管理

### Before Hook：初始化

```rust
#[before]
fn setup_world(world: &mut ConnectRuntimeWorld) {
    // 初始化业务状态
    world.task_id = 1;
    world.committed_seqno = 0;
    
    // 初始化测试基础设施
    let buffer = Arc::new(Mutex::new(Vec::new()));
    world.logs = Some(buffer.clone());
    
    // 设置tracing subscriber（见rust-tracing-isolation.md）
    setup_tracing(world);
}
```

### After Hook：清理 + 证据保存

```rust
#[after]
fn cleanup_and_save_evidence(world: &mut ConnectRuntimeWorld, scenario: &cucumber::Scenario) {
    // 保存证据（失败时）
    if scenario.failed() {
        save_evidence_manifest(world, scenario);
    }
    
    // 清理临时资源
    if let Some(temp_dir) = &world.temp_dir {
        std::fs::remove_dir_all(temp_dir).ok();
    }
    
    // Subscriber guard自动drop清理（无需手动）
}
```

### Drop实现（自动清理）

```rust
impl Drop for ConnectRuntimeWorld {
    fn drop(&mut self) {
        // 清理临时目录（如果After hook未执行）
        if let Some(temp_dir) = &self.temp_dir {
            std::fs::remove_dir_all(temp_dir).ok();
        }
        
        // tracing subscriber guard自动drop
    }
}
```

---

## 避免的反模式

### ❌ 反模式1：跨场景共享状态

```rust
// ❌ 错误：使用LazyLock共享状态
use std::sync::LazyLock;

static SHARED_STATE: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| {
    Mutex::new(HashMap::new())
});

#[given("some setup")]
fn step_setup(world: &mut World) {
    // ❌ 修改全局状态，影响其他场景
    SHARED_STATE.lock().unwrap().insert("key", "value");
}
```

**问题**：状态泄漏到后续场景，导致测试不可预测。

### ❌ 反模式2：World过于庞大

```rust
// ❌ 错误：World包含所有可能状态
#[derive(World)]
pub struct MegaWorld {
    // 240+字段（你的当前实现）
    pub field1: String,
    pub field2: i32,
    // ...所有模块的所有状态
}
```

**问题**：
- 初始化开销大
- 序列化输出bloat
- 失败诊断信息冗余

### ✅ 正确：按模块拆分World（可选）

```rust
// 如果项目过大，可考虑拆分
#[derive(World)]
pub struct MirrorMakerWorld {
    pub checkpoint_state: CheckpointState,
    pub heartbeat_state: HeartbeatState,
    pub source_state: SourceState,
}

#[derive(Debug, Serialize)]
pub struct CheckpointState {
    pub offset_sync: HashMap<String, i64>,
    pub checkpoint_result: Option<i64>,
}
```

---

## 诊断增强建议

### 建议1：添加诊断元数据

```rust
#[derive(Debug, Serialize, cucumber::World)]
pub struct ConnectRuntimeWorld {
    // 诊断元数据
    #[serde(skip)]
    pub current_step: Option<String>,
    
    #[serde(skip)]
    pub last_successful_step: Option<String>,
    
    #[serde(skip)]
    pub step_history: Vec<String>,
}

#[given(regex = ".*")]
#[track_caller]  // ← 捕获调用位置
fn track_step(world: &mut ConnectRuntimeWorld, step: &str) {
    world.current_step = Some(step.to_string());
    world.step_history.push(step.to_string());
}
```

**输出示例**：
```json
{
  "world_state": {
    "current_step": "Then offset should be committed",
    "last_successful_step": "When poll 5 records",
    "step_history": [
      "Given task initialized",
      "When poll 5 records",
      "Then offset should be committed"
    ]
  }
}
```

### 建议2：添加断言历史

```rust
pub struct AssertionRecord {
    pub step: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
}

#[derive(World)]
pub struct ConnectRuntimeWorld {
    #[serde(skip)]
    pub assertions: Vec<AssertionRecord>,
}

// 在Then步骤中记录
#[then(regex = "offset should be committed")]
fn check_commit(world: &mut ConnectRuntimeWorld) {
    let expected = world.expected_committed;
    let actual = world.committed_seqno > 0;
    
    world.assertions.push(AssertionRecord {
        step: "offset should be committed",
        expected: format!("committed_seqno > 0"),
        actual: format!("committed_seqno = {}", world.committed_seqno),
        passed: actual,
    });
    
    assert!(actual);
}
```

---

## 参考资源

### 官方文档

- [cucumber-rs World trait](https://docs.rs/cucumber/latest/cucumber/trait.World.html)
- [cucumber-rs state management](https://cucumber-rs.github.io/cucumber/current/writing/state.html)
- [cucumber-rs event system](https://cucumber-rs.github.io/cucumber/current/architecture/)

### GitHub实现

- [cucumber-rs event.rs](https://github.com/cucumber-rs/cucumber/blob/main/src/event.rs) - Step::Failed with Arc<World>
- [cucumber-rs World examples](https://github.com/cucumber-rs/cucumber/tree/main/examples)
- [TypeDB HTTP tests World](https://github.com/typedb/typedb/blob/master/tests/behaviour/service/http/http_steps/lib.rs)

### 你的项目实现

- `connect-runtime/tests/bdd/world.rs` - 已有World实现
- `connect-mirror/tests/bdd/world.rs` - 已有World实现