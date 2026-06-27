# 条件编译改变 struct 布局 → const generic + cfg

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码用 `#ifdef` / `#if` 改变 struct 字段和 padding，on-flash 二进制布局随配置变化：

```c
// C: padding 随 FDB_WRITE_GRAN 变化
struct sector_hdr_data {
    uint8_t store[2];
    uint8_t dirty[2];
    uint32_t magic;
    uint32_t combined;
    uint32_t reserved;
#if (FDB_WRITE_GRAN == 64)
    uint8_t padding[4];
#elif (FDB_WRITE_GRAN == 128)
    uint8_t padding[12];
#elif (FDB_WRITE_GRAN == 256)
    uint8_t padding[28];
#endif
};

// C: 整个字段条件存在
struct fdb_db {
    ...
    union {
#ifdef FDB_USING_FAL_MODE
        const struct fal_partition *part;
#endif
#ifdef FDB_USING_FILE_MODE
        const char *dir;
#endif
    } storage;
#ifdef FDB_USING_FILE_MODE
    uint32_t cur_file_sec[2];
    int cur_file[2];
    uint32_t cur_sec;
#endif
    ...
};
```

## 解决方案

### 场景 A：数值型配置（如 FDB_WRITE_GRAN）→ const generic

```rust
// 用 const generic 参数化 WRITE_GRAN
#[repr(C)]
struct SectorHdrData<const WRITE_GRAN: u32> {
    store: [u8; 2],
    dirty: [u8; 2],
    magic: u32,
    combined: u32,
    reserved: u32,
    padding: <Self as SectorHdrPadding<WRITE_GRAN>>::Padding,
}

// 用 associated type 按配置选择 padding 类型
trait SectorHdrPadding<const GRAN: u32> {
    type Padding: Default + AsRef<[u8]> + AsMut<[u8]>;
}

impl SectorHdrPadding<1> for SectorHdrData<1> {
    type Padding = [u8; 0];  // GRAN=1 无 padding
}
impl SectorHdrPadding<8> for SectorHdrData<8> {
    type Padding = [u8; 0];  // GRAN=8 无 padding
}
impl SectorHdrPadding<64> for SectorHdrData<64> {
    type Padding = [u8; 4];
}
impl SectorHdrPadding<128> for SectorHdrData<128> {
    type Padding = [u8; 12];
}
impl SectorHdrPadding<256> for SectorHdrData<256> {
    type Padding = [u8; 28];
}
```

### 场景 B：布尔型 feature flag → `#[cfg(feature)]`

```rust
// Cargo.toml:
// [features]
// fal-mode = []
// file-mode = ["file-posix"]  # 或 file-libc，互斥用 build.rs 控制
// file-posix = []
// file-libc = []

#[repr(C)]
struct FdbDb {
    name: *const c_char,  // 或 &'static str 配合生命周期
    type_: FdbDbType,
    sec_size: u32,
    max_size: u32,
    oldest_addr: u32,
    init_ok: bool,
    file_mode: bool,
    not_formatable: bool,

    #[cfg(feature = "fal-mode")]
    storage_fal: Option<&'static FalPartition>,

    #[cfg(feature = "file-mode")]
    storage_dir: Option<&'static str>,

    #[cfg(feature = "file-mode")]
    cur_file_sec: [u32; FDB_FILE_CACHE_TABLE_SIZE],

    #[cfg(feature = "file-posix")]
    cur_file_posix: [i32; FDB_FILE_CACHE_TABLE_SIZE],

    #[cfg(feature = "file-libc")]
    cur_file_libc: [*mut FILE; FDB_FILE_CACHE_TABLE_SIZE],

    #[cfg(feature = "file-mode")]
    cur_sec: u32,

    lock: Option<extern "C" fn(*mut FdbDb)>,
    unlock: Option<extern "C" fn(*mut FdbDb)>,
    user_data: *mut c_void,
}
```

### 场景 C：运行时互斥（storage union）→ enum

```rust
// C 的 union storage → Rust enum（运行时选择，非编译时）
#[derive(Clone, Copy)]
enum Storage {
    Fal(&'static FalPartition),
    File(&'static str),
}

struct FdbDb {
    name: &'static str,
    storage: Storage,  // 运行时决定，非条件编译
    // ...
}
```

## on-flash 兼容性验证（强制）

**每种配置组合必须独立验证 `size_of` 和 `offset_of`**：

```rust
#[cfg(all(feature = "fal-mode", not(feature = "file-mode")))]
const _: () = {
    assert!(core::mem::size_of::<FdbDb>() == EXPECTED_FAL_SIZE);
    assert!(core::mem::offset_of!(FdbDb, sec_size) == EXPECTED_SEC_SIZE_OFFSET);
};

#[cfg(all(feature = "file-mode", not(feature = "fal-mode")))]
const _: () = {
    assert!(core::mem::size_of::<FdbDb>() == EXPECTED_FILE_SIZE);
    assert!(core::mem::offset_of!(FdbDb, sec_size) == EXPECTED_SEC_SIZE_OFFSET);
};
```

## ❌ 禁止模式

```rust
// 禁止：在同一 struct 上用 cfg 增删字段但不验证布局
#[repr(C)]
struct Bad {
    #[cfg(feature = "x")]
    field_a: u32,
    field_b: u32,  // field_b 的偏移随 feature 变化！
}

// 禁止：用运行时 if 替代编译时配置
struct Bad {
    padding: Vec<u8>,  // 运行时大小，非 #[repr(C)]
}
```

## 注意事项

- on-flash struct 的 `#[cfg]` 字段改变 `size_of`，影响 flash 布局兼容性
- 不同 feature 组合产生不兼容的 flash 格式——这是设计约束，不是 bug
- const generic 比 feature flag 更适合数值型配置（如 WRITE_GRAN），因为类型系统在编译时保证一致性
- `#[cfg]` 在 struct 字段上使用时，`#[derive(Default)]` 可能失败——需手动实现或用 const generic
