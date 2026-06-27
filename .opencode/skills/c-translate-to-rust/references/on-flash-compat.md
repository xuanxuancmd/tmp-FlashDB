# on-flash 格式兼容性验证

> 返回 [SKILL.md](../SKILL.md) §5 零容忍项

## 核心约束

FlashDB 的价值在于 on-flash 二进制格式。Rust 重写必须保持与 C 版本**字节级兼容**。

## 验证方法

### 1. 编译时 size_of 断言

```rust
// 每种 FDB_WRITE_GRAN 配置独立验证
#[repr(C)]
struct SectorHdrData {
    store: [u8; STORE_STATUS_TABLE_SIZE],
    dirty: [u8; DIRTY_STATUS_TABLE_SIZE],
    magic: u32,
    combined: u32,
    reserved: u32,
    #[cfg(FDB_WRITE_GRAN_64)]
    padding: [u8; 4],
    // ...
}

// 编译时验证
const _: () = assert!(core::mem::size_of::<SectorHdrData>() == EXPECTED_SIZE);
const _: () = assert!(core::mem::align_of::<SectorHdrData>() == EXPECTED_ALIGN);
```

### 2. 差异测试（Differential Testing）

```rust
// 编译 C 版本为 .so，Rust 版本为 .so，相同输入对比输出
#[test]
fn test_crc32_parity() {
    let inputs = [[0u8; 0], [0xFF; 1], [0x01; 4], [0xAB; 256]];
    for input in &inputs {
        let c_result = unsafe { c_fdb_calc_crc32(0, input.as_ptr(), input.len()) };
        let rust_result = fdb_calc_crc32(0, input);
        assert_eq!(c_result, rust_result, "CRC32 mismatch for input len {}", input.len());
    }
}

#[test]
fn test_status_table_parity() {
    for gran in [1, 8, 32, 64, 128, 256] {
        for status_num in 2..=8 {
            for status_index in 0..status_num {
                let mut c_table = [0u8; 32];
                let c_result = unsafe { c_fdb_set_status(c_table.as_mut_ptr(), status_num, status_index) };

                let mut rust_table = [0u8; 32];
                let rust_result = set_status(&mut rust_table, status_num, status_index);

                assert_eq!(c_table, rust_table, "Status table mismatch: gran={}, num={}, idx={}", gran, status_num, status_index);
            }
        }
    }
}
```

### 3. Flash 镜像互操作测试

```rust
// C 写入 flash → Rust 读取验证
#[test]
fn test_c_write_rust_read() {
    // 1. C 版本创建 flash 镜像
    unsafe { c_fdb_kvdb_init(&mut c_db, "test", "test_db", &default_kvs); }
    unsafe { c_fdb_kv_set_blob(&mut c_db, "key1", &make_blob(&[1, 2, 3])); }

    // 2. 读取 C 写入的 flash 镜像文件
    let flash_data = std::fs::read("test_db/test.fdb.0").unwrap();

    // 3. Rust 版本读取相同镜像
    let mut rust_db = FdbKvdb::init("test", "test_db", Config::default()).unwrap();

    // 4. 验证 Rust 能读出 C 写入的数据
    let blob = rust_db.kv_get_blob("key1").unwrap();
    assert_eq!(blob, &[1, 2, 3]);
}

// Rust 写入 → C 读取
#[test]
fn test_rust_write_c_read() {
    // 1. Rust 版本写入
    let mut rust_db = FdbKvdb::init("test", "test_db", Config::default()).unwrap();
    rust_db.kv_set_blob("key1", &[1, 2, 3]).unwrap();
    drop(rust_db);  // 确保写入完成

    // 2. C 版本读取
    let mut c_db: C_FdbKvdb = Default::default();
    unsafe { c_fdb_kvdb_init(&mut c_db, "test", "test_db", &default_kvs); }

    let mut buf = [0u8; 3];
    let blob = make_blob(&mut buf);
    let len = unsafe { c_fdb_kv_get_blob(&mut c_db, "key1", &blob) };
    assert_eq!(len, 3);
    assert_eq!(&buf, &[1, 2, 3]);
}
```

## 必须验证的 on-flash 元素

| 元素 | 验证方法 |
|------|----------|
| struct size 和 offset | `assert_eq!(size_of::<T>(), EXPECTED)` |
| magic word 值 | `assert_eq!(SECTOR_MAGIC_WORD, 0x30424446)` |
| CRC32 多项式 | 差异测试，10000+ 随机输入 |
| 状态表编码 | 差异测试，所有 gran × status_num × status_index 组合 |
| KV/TSL 节点布局 | flash 镜像互操作测试 |
| 扇区头布局 | flash 镜像互操作测试 |
| 字节序 | `#[cfg(feature = "big-endian")]` 分支测试 |
