# 嵌入式 Redis

Feature 涉及缓存/Redis 时加载此文件。

---

## 方案选型

| Crate | 类型 | 适用场景 |
|-------|------|----------|
| `redis` + `miniredis` | 内存模拟 | 快速单元测试，无需 Docker |
| `testcontainers` + Redis | Docker 容器 | 需要真实 Redis 协议 |

---

## redis-test / miniredis 模式

Rust 生态没有成熟的 miniredis 等价物，推荐两种方案：

### 方案1：testcontainers 启动真实 Redis

```toml
[dev-dependencies]
testcontainers = "0.23"
testcontainers-modules = { version = "0.11", features = ["redis"] }
redis = { version = "0.27", features = ["tokio-comp"] }
```

```rust
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::Redis;
use redis::AsyncCommands;

// 启动 Redis 容器
let container = Redis::default().start().await.unwrap();
let redis_url = format!(
    "redis://{}:{}/",
    container.get_host().unwrap(),
    container.get_host_port_ipv4(6379).unwrap()
);

// 连接
let client = redis::Client::open(redis_url).unwrap();
let mut conn = client.get_async_connection().await.unwrap();

// 写入
conn.set::<_, _, ()>("cache:key", "value").await.unwrap();

// 读取验证
let val: String = conn.get("cache:key").await.unwrap();
assert_eq!(val, "value");
// 容器 drop 时自动清理
```

### 方案2：Mock Redis 操作（轻量级）

如果只需要验证 Redis 操作是否被调用，可以用 `mockall` mock Redis client trait：

```rust
use mockall::mock;

mock! {
    pub RedisClient {}
    
    impl RedisClientTrait for RedisClient {
        async fn get(&self, key: &str) -> Result<Option<String>, RedisError>;
        async fn set(&self, key: &str, value: &str) -> Result<(), RedisError>;
        async fn del(&self, key: &str) -> Result<(), RedisError>;
    }
}
```

**注意**：方案2 属于 mock 内部实现，仅适用于单元测试。FT/E2E 测试应使用方案1。

---

## BDD Step 集成

### World 字段

```rust
#[derive(Debug, cucumber::World)]
#[world(init = Self::new)]
pub struct KafkaWorld {
    #[serde(skip)]
    pub redis_container: Option<testcontainers::ContainerAsync<Redis>>,
    #[serde(skip)]
    pub redis_client: Option<redis::Client>,
}

impl KafkaWorld {
    async fn new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            redis_container: None,
            redis_client: None,
            // ...
        })
    }
}
```

### Given — 启动嵌入式 Redis

```rust
#[given("a Redis instance")]
async fn given_redis(world: &mut KafkaWorld) {
    let container = Redis::default().start().await.unwrap();
    let redis_url = format!(
        "redis://{}:{}/",
        container.get_host().unwrap(),
        container.get_host_port_ipv4(6379).unwrap()
    );
    let client = redis::Client::open(redis_url).unwrap();
    world.redis_container = Some(container);
    world.redis_client = Some(client);
}
```

### Then — 验证 Redis 数据

```rust
#[then("the cache key {string} should have value {string}")]
async fn then_cache_value(world: &mut KafkaWorld, key: String, expected: String) {
    let client = world.redis_client.as_ref().unwrap();
    let mut conn = client.get_async_connection().await.unwrap();
    let actual: String = conn.get(&key).await
        .expect("Cache key should exist");
    assert_eq!(
        actual, expected,
        "Cache key '{}': expected '{}' but got '{}'",
        key, expected, actual
    );
}
```

---

## 资源清理

testcontainers 容器在 `drop` 时自动停止并删除。World 中持有 `Option<ContainerAsync<Redis>>`，Scenario 结束自动释放。

---

## 参考

- [testcontainers-modules Redis](https://github.com/testcontainers/testcontainers-rs/tree/main/testcontainers-modules)
- [redis crate](https://docs.rs/redis/latest/redis/)
