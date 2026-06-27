# static 可变全局变量 → Atomic / Mutex / OnceLock

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码用 `static` 可变全局变量管理初始化状态和设备注册表：

```c
// C: 可变全局状态
static uint8_t init_ok = 0;          // FAL 初始化标志
static const struct fal_partition *partition_table = NULL;  // 分区表指针
static struct part_flash_info part_flash_cache[10] = {0};   // 分区缓存
static uint8_t *data = rt_malloc(size);                      // 动态分配
static sfud_flash_t sfud_dev = NULL;                         // SFUD 设备
static int ef_err_port_cnt = 0;                              // 错误计数器

// C: 函数局部 static（返回缓冲区，非可重入）
static char value[FDB_STR_KV_VALUE_MAX_SIZE + 1];  // fdb_kv_get 返回
```

## 解决方案

### 场景 A：布尔/整数标志 → `AtomicBool` / `AtomicI32`

```rust
// C: static uint8_t init_ok = 0;
// Rust:
use std::sync::atomic::{AtomicBool, Ordering};

static INIT_OK: AtomicBool = AtomicBool::new(false);

fn fal_init() -> Result<(), FlashError> {
    if INIT_OK.load(Ordering::SeqCst) {
        return Ok(());  // 已初始化
    }
    // ... 初始化逻辑 ...
    INIT_OK.store(true, Ordering::SeqCst);
    Ok(())
}

// C: static int ef_err_port_cnt = 0;
// Rust:
static ERR_PORT_CNT: AtomicI32 = AtomicI32::new(0);

fn record_error() {
    ERR_PORT_CNT.fetch_add(1, Ordering::Relaxed);
}
```

### 场景 B：单次初始化 → `OnceLock<T>`

```rust
// C: static const struct fal_partition *partition_table = NULL; (init once)
// Rust:
use std::sync::OnceLock;

static PARTITION_TABLE: OnceLock<Vec<FalPartition>> = OnceLock::new();

fn fal_partition_init() -> Result<(), FlashError> {
    PARTITION_TABLE.get_or_init(|| {
        // 首次调用时初始化
        load_partition_table().unwrap_or_default()
    });
    Ok(())
}

fn find_partition(name: &str) -> Option<&'static FalPartition> {
    PARTITION_TABLE.get()?
        .iter()
        .find(|p| p.name == name)
}
```

### 场景 C：可变集合 → `Mutex<T>`

```rust
// C: static struct part_flash_info part_flash_cache[10] = {0};
// Rust:
use std::sync::Mutex;

static PART_FLASH_CACHE: Mutex<[PartFlashInfo; 10]> = Mutex::new([PartFlashInfo::default(); 10]);

fn update_cache(index: usize, info: PartFlashInfo) {
    let mut cache = PART_FLASH_CACHE.lock().unwrap();
    cache[index] = info;
}

// no_std 环境：
use critical_section::Mutex as CsMutex;
static PART_FLASH_CACHE: CsMutex<[PartFlashInfo; 10]> = CsMutex::new([PartFlashInfo::default(); 10]);
```

### 场景 D：函数局部 static（返回缓冲区）→ 改为返回拥有所有权的类型

```c
// C: 非可重入的 static buffer 返回
char *fdb_kv_get(fdb_kvdb_t db, const char *key) {
    static char value[FDB_STR_KV_VALUE_MAX_SIZE + 1];  // ← 非可重入！
    // ... 填充 value ...
    return value;  // 返回内部 static buffer 指针
}
```

```rust
// Rust: 返回拥有所有权的 String（线程安全）
impl FdbKvdb {
    pub fn kv_get(&self, key: &str) -> Option<String> {
        let mut buf = [0u8; FDB_STR_KV_VALUE_MAX_SIZE + 1];
        let len = self.get_kv(key, &mut buf)?;
        Some(String::from_utf8_lossy(&buf[..len]).into_owned())
    }
}

// 或在 no_std + no_alloc 环境中，返回到调用者提供的 buffer
impl FdbKvdb {
    pub fn kv_get<'a>(&self, key: &str, buf: &'a mut [u8]) -> Option<&'a str> {
        let len = self.get_kv(key, buf)?;
        Some(core::str::from_utf8(&buf[..len]).ok()?)
    }
}
```

### 场景 E：SFUD 设备指针 → `Mutex<Option<T>>`

```c
// C: static sfud_flash_t sfud_dev = NULL; (init 时设置)
static sfud_flash_t sfud_dev = NULL;

int init(void) {
    sfud_dev = &sfud_norflash0;  // 运行时设置
    sfud_dev->chip.erase_gran;   // 运行时访问
}
```

```rust
// Rust: 用 OnceLock 或 Mutex<Option<T>>
use std::sync::OnceLock;

static SFUD_DEV: OnceLock<SfudFlash> = OnceLock::new();

fn init() -> Result<(), FlashError> {
    let dev = SfudFlash::probe()?;
    SFUD_DEV.set(dev).ok();  // 首次设置，后续忽略
    Ok(())
}

fn read(offset: u32, buf: &mut [u8]) -> Result<usize, FlashError> {
    let dev = SFUD_DEV.get().ok_or(FlashError::NotInitialized)?;
    dev.read(offset, buf)
}
```

### 场景 F：const 设备表 → `&'static [T]`

```c
// C: static const struct fal_flash_dev * const device_table[] = FAL_FLASH_DEV_TABLE;
```

```rust
// Rust: 编译时常量数组
static DEVICE_TABLE: &[&dyn ErasedFlashDevice] = &[
    &Stm32F4Flash::new(0x08000000, 1024 * 1024, 128 * 1024),
    // &NorFlash0::new(...),
];

// 使用
fn find_device(name: &str) -> Option<&'static dyn ErasedFlashDevice> {
    DEVICE_TABLE.iter()
        .find(|dev| dev.name() == name)
        .copied()
}
```

## ❌ 禁止模式

```rust
// 禁止：static mut（Rust 2024 已弃用）
static mut INIT_OK: bool = false;
// 原因：数据竞争风险；用 AtomicBool 替代

// 禁止：函数局部 static 返回缓冲区
fn kv_get(&self, key: &str) -> &str {
    static VALUE: Mutex<String> = Mutex::new(String::new());
    let mut val = VALUE.lock().unwrap();
    val.clear();
    val.push_str("...");
    &val  // 返回锁守卫的引用——编译错误或死锁
}
// 原因：非可重入，线程不安全；返回拥有所有权的 String
```

## 注意事项

- `AtomicBool`/`AtomicI32` 适合简单的标志和计数器
- `OnceLock<T>` 适合单次初始化的全局状态（Rust 1.70+）
- `Mutex<T>` 适合可变集合（`std::sync::Mutex` 在 std；`critical_section::Mutex` 在 no_std）
- 函数局部 static 返回 buffer 是**非可重入**的——必须改为返回拥有所有权的类型
- `lazy_static!` 和 `once_cell` crate 已被 std 的 `OnceLock`/`LazyLock` 取代（Rust 1.70/1.80+）
- no_std 环境用 `critical_section::Mutex` 替代 `std::sync::Mutex`
