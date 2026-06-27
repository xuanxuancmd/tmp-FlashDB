# HTTP Mock Server — wiremock

Feature 涉及外部 REST API 时加载此文件。

---

## Crate

```toml
[dev-dependencies]
wiremock = "0.6"
```

`wiremock` 是 Rust 生态的 WireMock 等价物，每个测试独立 MockServer，随机端口，完全隔离。

---

## 基本用法

```rust
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path, body_json};

// 每个测试独立 MockServer
let mock_server = MockServer::start().await;

// 定义 mock 行为
Mock::given(method("GET"))
    .and(path("/connectors"))
    .respond_with(ResponseTemplate::new(200)
        .set_body_json(serde_json::json!([
            {"name": "file-source", "type": "source"}
        ])))
    .expect(1)  // 期望恰好被调用 1 次
    .named("list-connectors")  // 命名用于调试
    .mount(&mock_server)
    .await;

// 业务代码使用 mock_server.uri() 作为 base URL
let base_url = mock_server.uri();  // e.g. "http://127.0.0.1:43721"
```

---

## Scope 控制 — 精确生命周期

```rust
// Mock 仅在 guard 存活期间有效
let guard = Mock::given(method("POST"))
    .and(path("/connectors"))
    .respond_with(ResponseTemplate::new(201))
    .mount_as_scoped(&mock_server)
    .await;

// ... 执行测试 ...
drop(guard);  // Mock 立即消失，同时验证 expect 断言
```

---

## BDD Step 集成

### World 字段

```rust
#[derive(Debug, cucumber::World)]
#[world(init = Self::new)]
pub struct KafkaWorld {
    #[serde(skip)]
    pub mock_server: wiremock::MockServer,
    // ...
}

impl KafkaWorld {
    async fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            mock_server: wiremock::MockServer::start().await,
            // ...
        })
    }
}
```

### Given — 配置外部 API Mock

```rust
#[given("a REST mock for connector status returning {int}")]
async fn given_mock_connector_status(world: &mut KafkaWorld, status: u16) {
    Mock::given(method("GET"))
        .and(path("/connectors/my-connector/status"))
        .respond_with(ResponseTemplate::new(status)
            .set_body_json(serde_json::json!({
                "name": "my-connector",
                "connector": {"state": "RUNNING"},
                "tasks": [{"id": 0, "state": "RUNNING"}]
            })))
        .mount(&world.mock_server)
        .await;
}
```

### Then — 验证外部 API 调用

```rust
#[then("the REST API should have been called {int} time(s)")]
async fn then_api_called(world: &mut KafkaWorld, _expected: usize) {
    // wiremock 的 expect(N) 在 MockServer drop 时自动验证
    // 显式验证：
    world.mock_server.verify().await;
}
```

---

## 资源清理

MockServer 在 `drop` 时自动验证所有 `expect()` 断言并清理。无需手动 shutdown。

---

## 参考

- [wiremock crate](https://docs.rs/wiremock/latest/wiremock/)
- [wiremock examples](https://github.com/LukeMathWalker/wiremock-rs/tree/master/examples)
