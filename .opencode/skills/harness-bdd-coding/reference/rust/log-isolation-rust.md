# Rust日志隔离采集技术方案

本文档提供Rust测试环境中日志隔离和采集的具体技术实现方案，适用于Harness工程的BDD测试场景。

## 技术选型对比

| 方案 | 适用场景 | 捕获范围 | 推荐度 |
|------|----------|----------|--------|
| **Thread-Local Subscriber** | 单线程测试 | tracing/log | ⭐⭐⭐ |
| **Global + Span Filter** | 异步测试、spawn任务 | tracing（跨线程） | ⭐⭐⭐⭐ |
| **with_test_writer()** | tracing + println组合 | tracing + println! | ⭐⭐⭐⭐⭐ |
| **Process Isolation** | 完全隔离 | 全局状态 | ⭐⭐⭐⭐ |

---

## 方案1：Thread-Local Subscriber（单线程）

### 原理

使用`tracing::subscriber::set_default()`在当前线程设置subscriber，作用域结束时自动清理。

### 实现代码

```rust
use tracing_subscriber::fmt;
use std::sync::{Arc, Mutex};

// MockWriter：写入内存buffer
pub struct MemoryWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl std::io::Write for MemoryWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
}

// World中存储日志
#[derive(cucumber::World)]
pub struct TestWorld {
    #[serde(skip)]
    logs: Arc<Mutex<Vec<u8>>>,
    
    #[serde(skip)]
    subscriber_guard: Option<tracing::subscriber::DefaultGuard>,
}

// Before hook：设置subscriber
#[before]
fn setup_logging(world: &mut TestWorld) {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = MemoryWriter { buffer: buffer.clone() };
    
    // set_default()返回guard，drop时清理
    let guard = fmt()
        .with_writer(writer)
        .with_max_level(tracing::Level::DEBUG)
        .set_default();  // ← 线程局部
    
    world.logs = buffer;
    world.subscriber_guard = Some(guard);  // ← 保持guard存活
}

// After hook：提取日志
#[after]
fn save_logs(world: &mut TestWorld, scenario: &cucumber::Scenario) {
    if scenario.failed() {
        let logs = world.logs.lock().unwrap().clone();
        let content = String::from_utf8_lossy(&logs);
        
        // 写入证据文件
        std::fs::write(
            format!("evidence/{}/logs.txt", scenario.name),
            content.as_ref()
        );
    }
    // Guard在World drop时自动清理subscriber
}
```

### 关键点

- `set_default()` vs `set_global_default()`：
  - `set_default()` → 线程局部，返回guard，scope结束自动清理
  - `set_global_default()` → 进程全局，只能设置一次

- **限制**：无法捕获spawn任务中的日志（线程局部特性）

---

## 方案2：Global Subscriber + Span Filter（异步）

### 原理

使用全局subscriber + 每场景独立的span名，通过span过滤日志。

### 实现代码

```rust
use tracing::{info_span, Span};
use tracing_subscriber::{fmt, EnvFilter, Registry};
use tracing_subscriber::layer::SubscriberExt;

// 全局buffer（静态）
static GLOBAL_LOGS: std::sync::OnceLock<std::sync::Mutex<Vec<(String, String)>>> = 
    std::sync::OnceLock::new();

// Layer记录日志+span名
struct SpanAwareLayer {
    buffer: &'static std::sync::Mutex<Vec<(String, String)>>,
}

impl tracing_subscriber::Layer<Registry> for SpanAwareLayer {
    fn on_event(&self, event: &tracing::Event, ctx: tracing_subscriber::layer::Context<'_, Registry>) {
        // 获取当前span名
        let span_name = ctx.current_span().name().unwrap_or("unknown");
        
        // 格式化日志内容
        let mut buf = String::new();
        tracing_subscriber::fmt::format::Format::default()
            .format_event(ctx.metadata(), event, &mut buf);
        
        self.buffer.lock().unwrap().push((span_name.to_string(), buf));
    }
}

// Before hook：创建场景span
#[before]
fn setup_tracing(world: &mut TestWorld, scenario: &cucumber::Scenario) {
    // 全局subscriber初始化（仅一次）
    GLOBAL_LOGS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    
    // 创建场景专属span
    let span = info_span!("scenario:{}", scenario.name);
    world.current_span = span.clone();
    world.span_guard = Some(span.enter());  // ← 进入span
}

// After hook：过滤获取本场景日志
#[after]
fn save_scenario_logs(world: &mut TestWorld, scenario: &cucumber::Scenario) {
    if scenario.failed() {
        let span_name = format!("scenario:{}", scenario.name);
        let buffer = GLOBAL_LOGS.get().unwrap();
        let all_logs = buffer.lock().unwrap().clone();
        
        // 过滤本场景的日志
        let scenario_logs: Vec<String> = all_logs
            .into_iter()
            .filter(|(name, _)| name == &span_name)
            .map(|(_, log)| log)
            .collect();
        
        std::fs::write(
            format!("evidence/{}/logs.txt", scenario.name),
            scenario_logs.join("\n")
        );
    }
}
```

### 关键点

- spawn任务中的日志会被捕获（全局subscriber）
- 每场景有独立span名，便于过滤
- cucumber-rs内置支持：`.init_tracing()`自动创建per-scenario span

---

## 方案3：with_test_writer（tracing + println组合）

### 原理

`tracing_subscriber::fmt().with_test_writer()` hook到libtest的OUTPUT_CAPTURE机制，同时捕获tracing和println。

### 实现代码

```rust
use tracing_subscriber::fmt;

// Before hook：启用test writer
#[before]
fn setup_test_capture(world: &mut TestWorld) {
    // with_test_writer()捕获tracing + println!
    let _ = fmt()
        .with_test_writer()  // ← 关键：hook到libtest
        .with_max_level(tracing::Level::DEBUG)
        .try_init();
}

// 注意：println!输出由libtest捕获，tracing输出到stdout（被libtest捕获）
// cargo test会自动捕获并显示在测试输出中
```

### 捕获范围

| 输出方式 | 是否捕获 | 原理 |
|----------|----------|------|
| `println!("msg")` | ✅ | libtest OUTPUT_CAPTURE |
| `eprintln!("msg")` | ✅ | libtest OUTPUT_CAPTURE |
| `log::info!("msg")` | ✅ | tracing → stdout → libtest |
| `tracing::info!("msg")` | ✅ | tracing → stdout → libtest |
| `std::io::stdout().write_all()` | ❌ | 直接写FD，绕过capture |

### 建议

- connect代码继续使用`log::info!`和`println!`（都会捕获）
- 避免`std::io::stdout().write_all()`（测试环境不捕获）

---

## 方案4：tracing-test Crate（生产级）

### 使用方式

```rust
// Cargo.toml
[dev-dependencies]
tracing-test = "0.2"

// 测试代码
#[tracing_test::traced_test]
#[tokio::test]
async fn test_offset_commit() {
    // spawn任务日志也会捕获（全局subscriber）
    tokio::spawn(async {
        tracing::info!("from spawned task");
    }).await.unwrap();
    
    // 验证日志
    assert!(logs_contain("from spawned task"));
}
```

### 实现原理

1. 全局subscriber + global buffer
2. 每测试自动创建span名（`test_fn_name`）
3. `logs_contain()`函数过滤检查日志

### 适用场景

- 异步测试
- spawn任务
- 生产级可靠性（24M+ downloads）

---

## 方案5：Process Isolation（nextest）

### 使用方式

```bash
# 安装
cargo install cargo-nextest

# 运行（每测试独立进程）
cargo nextest run
```

### 原理

- 每个测试在独立进程中运行
- stdout/stderr自然隔离
- 全局状态（logger、环境变量）完全隔离

### 优点

- 无需代码修改
- 无线程局部限制
- 完全隔离（最彻底）

### 缺点

- 需要额外工具
- 启动开销（进程创建）

---

## 推荐方案（针对Harness）

### 场景1：简单单线程BDD测试

**方案**：Thread-Local Subscriber + MemoryWriter

**优点**：
- 简单易实现
- 自动清理（guard机制）
- 不侵入业务代码

### 场景2：异步测试 + spawn任务

**方案**：Global Subscriber + Span Filter（或cucumber `.init_tracing()`）

**优点**：
- 跨线程捕获
- cucumber-rs内置支持
- 每场景独立过滤

### 场景3：同时捕获tracing + println

**方案**：`with_test_writer()`

**优点**：
- 组合捕获
- 无需额外实现
- cargo test原生支持

---

## 证据输出格式

### JSON Evidence Manifest

```json
{
  "scenario_id": "offset-commit-restart",
  "timestamp": "2026-05-06T14:30:00Z",
  "artifacts": [
    {
      "type": "logs",
      "format": "text",
      "content": "[INFO] Polling records...\n[ERROR] Commit timeout"
    },
    {
      "type": "world_state",
      "format": "json",
      "content": { "committed_seqno": 0, "poll_count": 5 }
    },
    {
      "type": "stack_trace",
      "format": "text",
      "content": "AssertionError at tests/bdd/offset_steps.rs:78"
    }
  ],
  "chain_of_custody": {
    "test_run_id": "ci-12345",
    "commit_sha": "abc123"
  }
}
```

### 实现示例

```rust
use serde_json::json;

#[after]
fn save_evidence_manifest(world: &mut TestWorld, scenario: &cucumber::Scenario) {
    if scenario.failed() {
        let logs = world.logs.lock().unwrap().clone();
        let logs_text = String::from_utf8_lossy(&logs);
        
        let manifest = json!({
            "scenario_id": scenario.name,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "artifacts": [
                json!({
                    "type": "logs",
                    "format": "text",
                    "content": logs_text.as_ref()
                }),
                json!({
                    "type": "world_state",
                    "format": "json",
                    "content": serde_json::to_value(&world).unwrap()
                }),
            ],
            "chain_of_custody": {
                "test_run_id": std::env::var("CI_JOB_ID").unwrap_or("local"),
                "commit_sha": std::env::var("GIT_COMMIT_SHA").unwrap_or("unknown")
            }
        });
        
        std::fs::write(
            format!("evidence/{}/manifest.json", scenario.name),
            serde_json::to_string(&manifest).unwrap()
        );
    }
}
```

---

## 参考资源

### GitHub实现示例

- [tracing-test crate](https://github.com/dbrgn/tracing-test) - 24M+ downloads
- [testing_logger crate](https://github.com/brucechapman/rust_testing_logger) - 3M+ downloads
- [tokio-rs/tracing dispatcher.rs](https://github.com/tokio-rs/tracing/blob/main/tracing-core/src/dispatcher.rs)
- [rust std OUTPUT_CAPTURE](https://github.com/rust-lang/rust/blob/main/library/std/src/io/stdio.rs)

### 官方文档

- [tracing subscriber docs](https://tracing.rs/tracing-subscriber/)
- [tracing::dispatcher::set_default](https://docs.rs/tracing-core/latest/tracing_core/dispatcher/fn.set_default.html)
- [cucumber-rs tracing integration](https://cucumber-rs.github.io/cucumber/current/output/tracing.html)

### 技术限制说明

- [rust-lang/rust issue #90785](https://github.com/rust-lang/rust/issues/90785) - stdout capture limitations
- Thread-local不传播到spawn线程（tracing-core文档）