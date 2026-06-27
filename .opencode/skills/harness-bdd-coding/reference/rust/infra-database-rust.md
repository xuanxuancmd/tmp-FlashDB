# 嵌入式数据库

Feature 涉及数据库读写时加载此文件。

---

## 方案选型

| Crate | 类型 | 适用场景 |
|-------|------|----------|
| `sled` | 嵌入式 KV 存储 | offset 存储、状态持久化 |
| `redb` | 嵌入式 KV（更现代） | 需要事务的 KV 场景 |
| `sqlx` + SQLite | 嵌入式 SQL | 关系型数据验证 |
| `testcontainers` + PostgreSQL | Docker 容器 | 需要真实 PostgreSQL 协议 |

---

## sled — 嵌入式 KV 存储

### Crate

```toml
[dev-dependencies]
sled = "0.34"
tempfile = "3"
```

### 基本用法

```rust
use sled::Db;
use tempfile::TempDir;

// 创建临时目录，测试结束自动清理
let temp_dir = TempDir::new().unwrap();
let db: Db = sled::open(temp_dir.path().join("test.db")).unwrap();

// 写入数据
db.insert("offset:topic-0:partition-0", &5i64.to_be_bytes()).unwrap();

// 读取数据
let value = db.get("offset:topic-0:partition-0").unwrap().unwrap();
let offset = i64::from_be_bytes(value.as_ref().try_into().unwrap());
assert_eq!(offset, 5);
```

### BDD Step 集成

```rust
#[derive(Debug, cucumber::World)]
pub struct KafkaWorld {
    #[serde(skip)]
    pub offset_db: Option<sled::Db>,
    #[serde(skip)]
    pub temp_dir: Option<tempfile::TempDir>,
}

#[given("an embedded offset store")]
async fn given_offset_store(world: &mut KafkaWorld) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = sled::open(temp_dir.path().join("offsets.db")).unwrap();
    world.offset_db = Some(db);
    world.temp_dir = Some(temp_dir);
}

#[then("the offset for partition {int} should be {int}")]
async fn then_offset_is(world: &mut KafkaWorld, partition: i32, expected: i64) {
    let db = world.offset_db.as_ref().unwrap();
    let key = format!("offset:topic-0:partition-{}", partition);
    let value = db.get(key.as_bytes()).unwrap()
        .expect("Offset should exist");
    let actual = i64::from_be_bytes(value.as_ref().try_into().unwrap());
    assert_eq!(
        actual, expected,
        "Partition {} offset: expected {} but got {}",
        partition, expected, actual
    );
}
```

---

## sqlx + SQLite — 嵌入式 SQL

### Crate

```toml
[dev-dependencies]
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio"] }
```

### 基本用法

```rust
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

// 内存 SQLite（每个测试独立）
let pool: SqlitePool = SqlitePoolOptions::new()
    .connect("sqlite::memory:")
    .await
    .unwrap();

// 建表
sqlx::query("CREATE TABLE offsets (topic TEXT, partition INT, offset_val BIGINT, PRIMARY KEY (topic, partition))")
    .execute(&pool)
    .await
    .unwrap();

// 插入
sqlx::query("INSERT INTO offsets (topic, partition, offset_val) VALUES (?, ?, ?)")
    .bind("test-topic")
    .bind(0i32)
    .bind(5i64)
    .execute(&pool)
    .await
    .unwrap();

// 查询验证
let row: (i64,) = sqlx::query_as("SELECT offset_val FROM offsets WHERE topic = ? AND partition = ?")
    .bind("test-topic")
    .bind(0i32)
    .fetch_one(&pool)
    .await
    .unwrap();
assert_eq!(row.0, 5);
```

---

## `build()` vs `create()`

| 方法 | 用途 | 场景 |
|------|------|------|
| `build()` | 创建内存对象 | 单元测试 |
| `create()` | 持久化到 DB | FT 测试，SUT 需查询到数据 |

```rust
impl ConnectorConfigBuilder {
    pub fn build(self) -> ConnectorConfig { self.config }
    
    pub async fn create(self, db: &sled::Db) -> ConnectorConfig {
        let config = self.build();
        db.insert(
            format!("connector:{}", config.name),
            serde_json::to_vec(&config).unwrap()
        ).unwrap();
        config
    }
}
```

---

## 资源清理

- `sled` + `TempDir`：TempDir drop 时自动删除临时目录
- `sqlite::memory:`：Pool drop 时自动释放
- `testcontainers`：Container drop 时自动停止并删除

---

## 参考

- [sled crate](https://docs.rs/sled/latest/sled/)
- [sqlx SQLite](https://docs.rs/sqlx/latest/sqlx/sqlite/index.html)
- [testcontainers-rs](https://github.com/testcontainers/testcontainers-rs)
