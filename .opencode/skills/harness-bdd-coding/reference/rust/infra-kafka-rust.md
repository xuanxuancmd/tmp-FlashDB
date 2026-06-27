# 嵌入式 Kafka — MockCluster（rdkafka）

Feature 涉及 Kafka/消息队列时加载此文件。

---

## Crate

```toml
[dev-dependencies]
rdkafka = { version = "0.36", features = ["cmake-build"] }
```

`MockCluster` 在进程内模拟多 Broker Kafka 集群，无需 Docker，启动快。

---

## 基本用法

```rust
use rdkafka::mock::MockCluster;
use rdkafka::producer::FutureProducer;
use rdkafka::consumer::StreamConsumer;
use rdkafka::ClientConfig;

// 创建 3 个模拟 broker
let mock_cluster = MockCluster::new(3).unwrap();

// 创建 topic
mock_cluster
    .create_topic("test-topic", 4, 3)  // topic, partitions, replication
    .expect("Failed to create topic");

// Producer
let producer: FutureProducer = ClientConfig::new()
    .set("bootstrap.servers", mock_cluster.bootstrap_servers())
    .set("message.timeout.ms", "5000")
    .create()
    .expect("Producer creation failed");

// Consumer
let consumer: StreamConsumer = ClientConfig::new()
    .set("bootstrap.servers", mock_cluster.bootstrap_servers())
    .set("group.id", "test-consumer")
    .set("auto.offset.reset", "earliest")  // ← 必须 earliest
    .create()
    .expect("Consumer creation failed");
```

---

## BDD Step 集成

### World 字段

```rust
#[derive(Debug, Default, cucumber::World)]
pub struct KafkaWorld {
    #[serde(skip)]
    pub mock_cluster: Option<MockCluster<'static, DefaultProducerContext>>,
    #[serde(skip)]
    pub kafka_producer: Option<FutureProducer>,
    #[serde(skip)]
    pub kafka_consumer: Option<StreamConsumer>,
    pub topic_name: Option<String>,
    pub messages_sent: usize,
}
```

### Given — 预置 Kafka 基础设施

```rust
#[given("a Kafka cluster with topic {string}")]
async fn given_kafka_topic(world: &mut KafkaWorld, topic: String) {
    let cluster = MockCluster::new(3).unwrap();
    cluster.create_topic(&topic, 4, 3).expect("create topic");
    
    let bs = cluster.bootstrap_servers();
    world.kafka_producer = Some(
        ClientConfig::new()
            .set("bootstrap.servers", &bs)
            .create::<FutureProducer>()
            .unwrap()
    );
    world.kafka_consumer = Some(
        ClientConfig::new()
            .set("bootstrap.servers", &bs)
            .set("group.id", "bdd-consumer")
            .set("auto.offset.reset", "earliest")
            .create::<StreamConsumer>()
            .unwrap()
    );
    
    world.mock_cluster = Some(cluster);
    world.topic_name = Some(topic);
}
```

### When — 发送真实消息

```rust
#[when("a record is produced to topic {string}")]
async fn when_record_produced(world: &mut KafkaWorld, topic: String) {
    let record = ConnectRecordBuilder::new()
        .topic(&topic)
        .key_string("test-key")
        .value_json(serde_json::json!({"data": "test-value"}))
        .build();
    world.produce_record(record).await;
}
```

### Then — 验证消息消费

```rust
#[then("the message should be consumed within {int} seconds")]
async fn then_message_consumed(world: &mut KafkaWorld, timeout_secs: u64) {
    let consumer = world.kafka_consumer.as_ref().unwrap();
    let msg = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        consumer.recv()
    ).await.expect("timeout waiting for message");
    
    let msg = msg.expect("kafka recv error");
    assert_eq!(
        msg.topic(),
        world.topic_name.as_ref().unwrap(),
        "Message should be from topic {}",
        world.topic_name.as_ref().unwrap()
    );
}
```

---

## 测试数据 Builder

```rust
pub struct ConnectRecordBuilder {
    topic: String,
    key: Option<String>,
    value: Option<serde_json::Value>,
    headers: Vec<(String, String)>,
}

impl ConnectRecordBuilder {
    pub fn new() -> Self {
        Self { topic: "test-topic".into(), key: None, value: None, headers: vec![] }
    }
    pub fn topic(mut self, topic: &str) -> Self { self.topic = topic.into(); self }
    pub fn key_string(mut self, key: &str) -> Self { self.key = Some(key.into()); self }
    pub fn value_json(mut self, json: serde_json::Value) -> Self { self.value = Some(json); self }
    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.into(), value.into())); self
    }
    pub fn build(self) -> ConnectRecord { /* ... */ }
}
```

---

## 资源清理

MockCluster 在 `drop` 时自动清理。World 中持有 `Option<MockCluster>`，Scenario 结束自动释放。

---

## 参考

- [rdkafka MockCluster](https://docs.rs/rdkafka/latest/rdkafka/mock/struct.MockCluster.html)
- [fede1024/rust-rdkafka mocking.rs](https://github.com/fede1024/rust-rdkafka/blob/master/examples/mocking.rs)
- [apache/iggy BDD tests](https://github.com/apache/iggy/tree/master/bdd/rust)
