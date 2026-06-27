# 测试迁移：RT-Thread Utest → #[test] + 差异测试

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 测试使用 RT-Thread Utest 框架，依赖 QEMU + RT-Thread BSP 运行：

```c
// C: RT-Thread Utest 框架
UTEST_TC_EXPORT(testcase, "packages.system.flashdb.kvdb", utest_tc_init, utest_tc_cleanup, 20);

static void test_fdb_create_kv_blob(void) {
    UTEST_UNIT_RUN(test_fdb_kvdb_init);
    UTEST_UNIT_RUN(test_fdb_create_kv_blob);
}

static void test_fdb_create_kv_blob_body(void) {
    uassert_true(result == FDB_NO_ERR);
    uassert_int_equal(blob.saved.len, sizeof(read_tick));
    uassert_buf_equal(&tick, value_buf, sizeof(value_buf));
}
```

**不能直接迁移** — 必须用 Rust `#[test]` 重写测试胶水。

## 解决方案

### 1. 断言宏映射

| C (Utest) | Rust (`#[test]`) |
|-----------|------------------|
| `uassert_true(cond)` | `assert!(cond)` |
| `uassert_int_equal(a, b)` | `assert_eq!(a, b)` |
| `uassert_int_not_equal(a, b)` | `assert_ne!(a, b)` |
| `uassert_str_equal(a, b)` | `assert_eq!(a, b)` (on `&str`) |
| `uassert_buf_equal(a, b, len)` | `assert_eq!(&a[..len], &b[..len])` |
| `uassert_null(ptr)` | `assert!(ptr.is_null())` 或 `assert!(result.is_none())` |
| `uassert_not_null(ptr)` | `assert!(!ptr.is_null())` 或 `assert!(result.is_some())` |

### 2. 测试结构迁移

```rust
// C: UTEST_TC_EXPORT 注册 + UTEST_UNIT_RUN 顺序执行
// Rust: 每个测试函数加 #[test]

#[test]
fn test_fdb_create_kv_blob() {
    // 初始化
    let mut kvdb = setup_kvdb();

    // 创建 KV blob
    let tick: u32 = 12345;
    let result = kvdb.kv_set_blob("kv_blob_test", &tick.to_le_bytes());
    assert!(result.is_ok());

    // 读取验证
    let read_tick = kvdb.kv_get_blob("kv_blob_test").unwrap();
    assert_eq!(read_tick, tick.to_le_bytes());
}

#[test]
fn test_fdb_gc() {
    let mut kvdb = setup_kvdb();

    // 设置默认值
    kvdb.kv_set_default().unwrap();

    // 插入大量 KV 触发 GC
    for i in 0..8 {
        let key = format!("kv_{}", i);
        let value = vec![i as u8; TEST_KV_VALUE_LEN];
        kvdb.kv_set_blob(&key, &value).unwrap();
    }

    // 验证 GC 后数据完整性
    for i in 0..8 {
        let key = format!("kv_{}", i);
        let value = kvdb.kv_get_blob(&key).unwrap();
        assert_eq!(value, vec![i as u8; TEST_KV_VALUE_LEN]);
    }
}
```

### 3. 重启模拟

```c
// C: fdb_reboot() = deinit + init
static void fdb_reboot(void) {
    fdb_kvdb_deinit(&test_kvdb);
    test_fdb_kvdb_init_by_sector_num(4);
}
```

```rust
// Rust: 同样 deinit + init
fn reboot(kvdb: &mut FdbKvdb) {
    kvdb.deinit();
    *kvdb = FdbKvdb::init("fdb_kvdb1", "test_db", Config::default().sec_size(4096)).unwrap();
}

#[test]
fn test_reboot_persistence() {
    let mut kvdb = setup_kvdb();

    // 写入数据
    kvdb.kv_set("key", "value").unwrap();

    // 重启
    reboot(&mut kvdb);

    // 验证数据持久化
    let value = kvdb.kv_get("key").unwrap();
    assert_eq!(value, "value");
}
```

### 4. 编译时测试参数计算

C 测试用大量宏计算测试参数（适配不同 WRITE_GRAN）：

```c
// C: 编译时计算测试参数
#define _TKV_W           ((FDB_WRITE_GRAN + 7) / 8)
#define _TKV_KV_STATUS_SZ  FDB_STATUS_TABLE_SIZE(FDB_KV_STATUS_NUM)
#define _TKV_SEC_HDR_RAW_SZ  (FDB_STORE_STATUS_TABLE_SIZE + FDB_DIRTY_STATUS_TABLE_SIZE + sizeof(uint32_t) + sizeof(uint32_t) + sizeof(uint32_t))
```

```rust
// Rust: 用 const fn 计算相同参数
const fn tkv_w() -> usize { (FDB_WRITE_GRAN as usize + 7) / 8 }
const fn tkv_kv_status_sz() -> usize { status_table_size(FDB_KV_STATUS_NUM) }
const fn tkv_sec_hdr_raw_sz() -> usize {
    STORE_STATUS_TABLE_SIZE + DIRTY_STATUS_TABLE_SIZE
    + core::mem::size_of::<u32>() * 3
}

// 测试中使用
const TEST_KV_VALUE_LEN: usize = tkv_max_val_aligned();
const TEST_KVDB_SECTOR_SIZE: u32 = 4096;
const TEST_KVDB_SECTOR_NUM: usize = 4;
```

### 5. 测试框架胶水

```rust
// 测试初始化/清理（替代 utest_tc_init / utest_tc_cleanup）
mod test_utils {
    use super::*;

    pub fn setup_kvdb() -> FdbKvdb {
        // 清理旧数据
        cleanup_db("test_db");
        // 初始化
        FdbKvdb::init("fdb_kvdb1", "test_db",
            Config::default()
                .sec_size(4096)
                .max_size(4096 * 4)
                .file_mode(true)
        ).unwrap()
    }

    pub fn cleanup_db(path: &str) {
        let _ = std::fs::remove_dir_all(path);
    }

    pub fn setup_tsdb() -> FdbTsdb {
        cleanup_db("test_tsdb");
        FdbTsdb::init("fdb_tsdb1", "test_tsdb",
            Config::default().sec_size(4096),
            get_time,
            256,  // max_len
        ).unwrap()
    }

    fn get_time() -> i32 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i32
    }
}

// 每个测试用 setup_kvdb() / setup_tsdb() 初始化
// Drop trait 自动清理
```

### 6. 补充测试（C 版本未覆盖的 API）

C 测试有 5 个 API 未覆盖，Rust 必须补充：

```rust
#[test]
fn test_kvdb_check() {
    let mut kvdb = setup_kvdb();
    kvdb.kv_set("key", "value").unwrap();
    assert!(kvdb.check().is_ok());  // fdb_kvdb_check — C 未测
}

#[test]
fn test_tsl_iter_reverse() {
    let mut tsdb = setup_tsdb();
    for i in 0..10 {
        tsdb.tsl_append(&i.to_le_bytes()).unwrap();
    }

    // 反向迭代 — C 未测
    let mut results = Vec::new();
    tsdb.tsl_iter_reverse(|tsl| {
        results.push(tsl.time);
        false
    });

    // 验证降序
    for i in 1..results.len() {
        assert!(results[i - 1] >= results[i]);
    }
}

#[test]
fn test_tsl_append_with_ts() {
    let mut tsdb = setup_tsdb();
    tsdb.tsl_append_with_ts(&[1, 2, 3], 1000).unwrap();
    tsdb.tsl_append_with_ts(&[4, 5, 6], 500).unwrap();

    let count = tsdb.tsl_query_count(0, 2000, TslStatus::Write);
    assert_eq!(count, 2);
}
```

## ❌ 禁止模式

```rust
// 禁止：删除 C 版本测试的失败用例来"通过"
// 禁止：用 todo!() / unimplemented!() 跳过测试
// 禁止：hardcode 测试期望值而不验证
```

## 注意事项

- C 测试用 QEMU + RT-Thread 运行，Rust 测试用 `cargo test` 直接运行
- file mode 测试可直接移植（用 `std::fs` 模拟 flash）
- FAL mode 测试需要 mock `FlashDevice` trait
- CI 矩阵需覆盖 6 种 `FDB_WRITE_GRAN`（用 `#[cfg(feature)]` 或参数化测试）
