# cucumber-rs使用指南

本文档提供cucumber-rs框架在Rust项目中的具体使用方法，适用于Harness工程的BDD测试实现。

---

## 基本架构

### 目录结构

```
connect-runtime/
├── tests/
│   ├── cucumber.rs              # Test runner
│   ├── bdd/
│   │   ├── mod.rs
│   │   ├── world.rs             # World定义
│   │   ├── task_lifecycle_steps.rs
│   │   ├── offset_management_steps.rs
│   │   ├── config_validation_steps.rs
│   │   ├── error_handling_steps.rs
│   │   ├── worker_coordination_steps.rs
│   │   └── test_plugins_steps.rs
│   └── resources/
│       └── features/
│           ├── connect-runtime-task-lifecycle.feature
│           ├── connect-runtime-offset-management.feature
│           └── ...
└── Cargo.toml
```

### Cargo.toml配置

```toml
[dev-dependencies]
cucumber = "0.20"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
chrono = "0.4"

[[test]]
name = "cucumber"
harness = false  # ← 关键：使用自定义harness
```

---

## World定义

### 基本实现

```rust
// tests/bdd/world.rs
use cucumber::World;
use std::collections::HashMap;

#[derive(Debug, serde::Serialize, World)]
pub struct ConnectRuntimeWorld {
    // Task lifecycle fields
    pub task_id: i32,
    pub task_started: bool,
    pub task_failed: bool,
    
    // Offset management fields
    pub committed_seqno: u64,
    pub backing_store_configured: bool,
    
    // Config validation fields
    pub validation_success: bool,
    
    // Test infrastructure
    #[serde(skip)]
    pub logs: Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>>,
    
    #[serde(skip)]
    pub config: HashMap<String, String>,
}

impl Default for ConnectRuntimeWorld {
    fn default() -> Self {
        Self {
            task_id: 0,
            task_started: false,
            task_failed: false,
            committed_seqno: 0,
            backing_store_configured: false,
            validation_success: false,
            logs: None,
            config: HashMap::new(),
        }
    }
}
```

---

## Step Definition

### 基本Step

```rust
// tests/bdd/task_lifecycle_steps.rs
use cucumber::{given, when, then};

#[given("a task with ID {int}")]
fn given_task(world: &mut ConnectRuntimeWorld, id: i32) {
    world.task_id = id;
    world.task_started = false;
    world.task_failed = false;
}

#[when("the task is started")]
fn when_start_task(world: &mut ConnectRuntimeWorld) {
    world.task_started = true;
}

#[then("the task should be running")]
fn then_task_running(world: &mut ConnectRuntimeWorld) {
    assert!(world.task_started);
    assert!(!world.task_failed);
}
```

### 正则匹配Step

```rust
use cucumber::given;

#[given(regex = "^a connector named (.*)$")]
fn given_connector(world: &mut ConnectRuntimeWorld, name: String) {
    world.config.insert("connector.name", name);
}
```

### 参数类型转换

```rust
// 自定义参数类型
#[derive(Debug)]
struct TaskId(i32);

impl cucumber::Parameter for TaskId {
    fn from_str(s: &str) -> Result<Self, String> {
        s.parse::<i32>()
            .map(TaskId)
            .map_err(|e| e.to_string())
    }
}

#[given("task with ID {TaskId}")]
fn given_task_id(world: &mut ConnectRuntimeWorld, id: TaskId) {
    world.task_id = id.0;
}
```

---

## Hooks

### Before Hook

```rust
use cucumber::before;

#[before]  // 所有场景前执行
fn setup(world: &mut ConnectRuntimeWorld) {
    world.task_id = 0;
    world.task_started = false;
}

#[before(tag = "@slow")]  // 仅带@slow标签的场景
fn setup_slow_test(world: &mut ConnectRuntimeWorld) {
    world.config.insert("timeout", "60s");
}
```

### After Hook

```rust
use cucumber::after;

#[after]
fn cleanup(world: &mut ConnectRuntimeWorld, scenario: &cucumber::Scenario) {
    // 保存证据（失败时）
    if scenario.failed() {
        save_evidence(world, scenario);
    }
    
    // 清理资源
    world.config.clear();
}
```

### Step-level Hook

```rust
use cucumber::{after_step, before_step};

#[before_step]
fn before_each_step(world: &mut ConnectRuntimeWorld, step: &cucumber::Step) {
    // 记录步骤历史
    world.step_history.push(step.name.clone());
}

#[after_step]
fn after_each_step(world: &mut ConnectRuntimeWorld, step: &cucumber::Step) {
    if step.failed() {
        // 记录失败步骤
        world.failed_step = Some(step.name.clone());
    }
}
```

---

## Test Runner

### 基本Runner

```rust
// tests/cucumber.rs
mod bdd;

use bdd::ConnectRuntimeWorld;
use cucumber::World;

#[tokio::main]
async fn main() {
    ConnectRuntimeWorld::cucumber()
        .run_and_exit("./tests/resources/features")
        .await;
}
```

### 增强Runner（with tracing）

```rust
#[tokio::main]
async fn main() {
    ConnectRuntimeWorld::cucumber()
        .init_tracing()  // ← 启用tracing支持
        .with_writer(MyDiagnosticWriter::new())  // ← 自定义Writer
        .run_and_exit("./tests/resources/features")
        .await;
}
```

### Tag过滤

```rust
ConnectRuntimeWorld::cucumber()
    .filter_run("./tests/resources/features", |selector| {
        selector
            .include("@smoke")  // 只跑@smoke标签
            .exclude("@skip")   // 排除@skip标签
    })
    .await;
```

---

## Feature文件位置

### 从Harness specs拷贝

```bash
# 编码前：拷贝feature文件到测试目录
cp .opencode/harness/specs/*.feature tests/resources/features/

# 禁止在编码阶段修改feature文件
```

### Feature文件格式

```gherkin
# tests/resources/features/connect-runtime-task-lifecycle.feature
Feature: Task lifecycle management

  @Evidence @Scope(task=worker_source_task)
  Scenario: Task initializes successfully
    Given a task with ID 1
    When the task is started
    Then the task should be running
```

---

## 自定义Writer

### 完整实现

```rust
use cucumber::Writer;
use std::io::Write;

pub struct DiagnosticWriter {
    output: Vec<u8>,
}

impl Writer for DiagnosticWriter {
    type Cli = cucumber::cli::Empty;
    
    fn handle_scenario(&mut self, ev: cucumber::Scenario) {
        writeln!(self.output, "Scenario: {}", ev.name);
    }
    
    fn handle_step(&mut self, ev: cucumber::Step) {
        match ev.status {
            cucumber::StepStatus::Passed => {
                writeln!(self.output, "  ✓ {}", ev.name);
            }
            cucumber::StepStatus::Failed => {
                writeln!(self.output, "  ✗ {}", ev.name);
                writeln!(self.output, "    Error: {}", ev.error);
                writeln!(self.output, "    Location: {}", ev.location);
            }
            _ => {}
        }
    }
    
    fn output(&mut self, w: &mut dyn Write) {
        w.write_all(&self.output);
    }
}
```

---

## 常见问题

### Q: 如何捕获World状态？

**A**: cucumber-rs已内置。失败时`Arc<World>`保存到Step::Failed事件。使用自定义Writer提取。

### Q: 如何捕获日志？

**A**: 见`reference/rust-tracing-isolation.md`。推荐`with_test_writer()`或Thread-Local Subscriber。

### Q: Step定义位置如何记录？

**A**: cucumber-rs proc macro自动注入file!/line!/column!。失败输出自动包含。

### Q: 如何实现失败诊断？

**A**: 见`reference/rust-evidence-collection.md`。使用自定义Writer生成AI diagnostic report。

---

## 参考资源

### 官方文档

- [cucumber-rs docs](https://cucumber-rs.github.io/cucumber/current/)
- [cucumber-rs World trait](https://docs.rs/cucumber/latest/cucumber/trait.World.html)
- [cucumber-rs hooks](https://cucumber-rs.github.io/cucumber/current/writing/hooks.html)

### GitHub

- [cucumber-rs repository](https://github.com/cucumber-rs/cucumber)
- [cucumber-rs examples](https://github.com/cucumber-rs/cucumber/tree/main/examples)

### 你的项目实现

- `connect-runtime/tests/cucumber.rs` - 已有runner
- `connect-runtime/tests/bdd/*.rs` - 已有step definitions
- `connect-mirror/tests/bdd/*.rs` - 已有step definitions