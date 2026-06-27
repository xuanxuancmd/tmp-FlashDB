# #ifdef 矩阵 → Cargo features + const generic

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

FlashDB 有 15+ 个 feature flag，形成复杂的条件编译矩阵：

```c
// C: 15+ 个 feature flag
#define FDB_USING_KVDB
#define FDB_USING_TSDB
#define FDB_USING_FAL_MODE
// #define FDB_USING_FILE_POSIX_MODE
// #define FDB_USING_FILE_LIBC_MODE
#define FDB_WRITE_GRAN 1  // 1/8/32/64/128/256
// #define FDB_USING_TIMESTAMP_64BIT
// #define FDB_KV_AUTO_UPDATE
// #define FDB_BIG_ENDIAN
// #define FDB_TSDB_FIXED_BLOB_SIZE 4
```

最复杂的是 `FDB_WRITE_GRAN`（6 种值）和 `FDB_USING_TIMESTAMP_64BIT`（改变多个 struct 的 time 字段大小）。

## Feature Flag → Cargo Feature 映射表

| C #define | Cargo Feature | 类型 | 互斥? |
|-----------|--------------|------|-------|
| `FDB_USING_KVDB` | `kvdb` | bool | 否 |
| `FDB_USING_TSDB` | `tsdb` | bool | 否 |
| `FDB_USING_FAL_MODE` | `fal-mode` | bool | 与 file-* 互斥 |
| `FDB_USING_FILE_POSIX_MODE` | `file-posix` | bool | 与 fal-mode/file-libc 互斥 |
| `FDB_USING_FILE_LIBC_MODE` | `file-libc` | bool | 与 fal-mode/file-posix 互斥 |
| `FDB_WRITE_GRAN` | const generic `GRAN: u32` | 数值(1/8/32/64/128/256) | — |
| `FDB_USING_TIMESTAMP_64BIT` | `timestamp-64bit` | bool | 否 |
| `FDB_KV_AUTO_UPDATE` | `kv-auto-update` | bool | 否 |
| `FDB_BIG_ENDIAN` | `big-endian` | bool | 否 |
| `FDB_TSDB_FIXED_BLOB_SIZE` | const generic `FIXED_BLOB: u32` | 数值(0=禁用) | — |
| `FDB_DEBUG_ENABLE` | `debug` | bool | 否 |
| `FDB_USING_NATIVE_ASSERT` | `native-assert` | bool | 否 |

## Cargo.toml 设计

```toml
[package]
name = "flashdb"
version = "0.1.0"
edition = "2021"

[features]
default = ["kvdb", "tsdb"]

# 功能模块
kvdb = []
tsdb = []

# 存储后端（互斥，用 build.rs 验证）
fal-mode = []
file-mode = ["file-posix"]  # 派生 feature
file-posix = []
file-libc = []

# 配置选项
timestamp-64bit = []
kv-auto-update = ["kvdb"]
big-endian = []
debug = []
native-assert = []

# 默认配置
[dependencies]
zerocopy = { version = "0.8", features = ["derive"] }
# no_std 环境用 heapless
heapless = { version = "0.8", optional = true }

# 可选：embedded-storage 适配
embedded-storage = { version = "0.3", optional = true }
```

## 互斥 Feature 验证（build.rs）

```rust
// build.rs: 验证互斥 feature
fn main() {
    let fal_mode = cfg!(feature = "fal-mode");
    let file_posix = cfg!(feature = "file-posix");
    let file_libc = cfg!(feature = "file-libc");

    // 存储后端必须选且只选一个
    let storage_count = [fal_mode, file_posix, file_libc].iter().filter(|&&x| x).count();
    if storage_count != 1 {
        panic!("Exactly one storage mode must be enabled: fal-mode, file-posix, or file-libc");
    }

    // file-posix 和 file-libc 互斥
    if file_posix && file_libc {
        panic!("file-posix and file-libc are mutually exclusive");
    }
}
```

## const generic 参数化

```rust
// FDB_WRITE_GRAN 用 const generic
#[repr(C)]
struct FlashDb<const GRAN: u32> {
    // 内部状态
}

impl<const GRAN: u32> FlashDb<GRAN> {
    const WRITE_GRAN: u32 = GRAN;

    // 编译时验证 GRAN 合法值
    const _: () = {
        assert!(matches!(GRAN, 1 | 8 | 32 | 64 | 128 | 256));
    };

    // 对齐计算（替代 C 的 FDB_WG_ALIGN 宏）
    const fn wg_align(size: usize) -> usize {
        let align = (GRAN as usize + 7) / 8;
        (size + align - 1) / align * align
    }

    // 状态表大小（替代 FDB_STATUS_TABLE_SIZE 宏）
    const fn status_table_size(status_num: usize) -> usize {
        if GRAN == 1 {
            (status_num * 1 + 7) / 8
        } else {
            ((status_num - 1) * GRAN as usize + 7) / 8
        }
    }
}

// 使用：实例化时指定 GRAN
let db: FlashDb<8> = FlashDb::new();  // STM32F2/F4
let db: FlashDb<1> = FlashDb::new();  // NOR flash
```

## 条件代码路径

```rust
// C: #ifdef FDB_USING_KVDB ... #endif
// Rust:
#[cfg(feature = "kvdb")]
pub mod kvdb;

#[cfg(feature = "tsdb")]
pub mod tsdb;

#[cfg(feature = "fal-mode")]
pub mod fal;

#[cfg(feature = "file-posix")]
pub mod file_posix;

#[cfg(feature = "file-libc")]
pub mod file_libc;

// 派生 feature
#[cfg(any(feature = "file-posix", feature = "file-libc"))]
pub mod file_mode;
```

## on-flash 格式兼容性警告

**关键**：以下配置改变 on-flash 二进制格式，不同配置不兼容：

| 配置 | 影响 | 不兼容场景 |
|------|------|-----------|
| `FDB_WRITE_GRAN` | struct padding 变化 | GRAN=8 写入的数据 GRAN=64 无法读取 |
| `FDB_USING_TIMESTAMP_64BIT` | time 字段 4→8 字节 | 32-bit 时间戳的 TSL 64-bit 无法解析 |
| `FDB_TSDB_FIXED_BLOB_SIZE` | log_idx_data 删除 log_len/log_addr | 固定 blob 和变长 blob 不兼容 |
| `FDB_BIG_ENDIAN` | magic word 字节序 | 大端写入的数据小端无法解析 |

这些配置在 Rust 中也必须是**编译时确定**的，不能运行时切换。

## ❌ 禁止模式

```rust
// 禁止：用运行时 if 替代编译时配置
struct FlashDb {
    write_gran: u32,  // 运行时值，非编译时
}
if db.write_gran == 64 { ... }  // 运行时分支，影响性能

// 禁止：Cargo feature 非加性（additive）
// features 应该是"添加功能"，不能"移除功能"
[features]
bad-feature = []  # 移除某些代码——反模式
```

## 注意事项

- Cargo features 是 additive 的——不能声明互斥；用 build.rs 验证
- const generic 是编译时单态化——每种 GRAN 生成独立类型，类型安全
- `#[cfg(feature)]` 在 `struct` 字段上使用会导致 `#[derive(Default)]` 失败——需手动实现
- 测试时需覆盖所有 feature 组合——用 CI matrix 测试
