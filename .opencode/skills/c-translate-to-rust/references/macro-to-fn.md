# do-while(0) 宏 → 函数 + Result

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 用 `do { ... } while(0)` 宏封装多语句逻辑，部分宏内含 `return` 语句：

```c
// C: 无 return 的 do-while(0) 宏
#define db_lock(db) do { \
    if (((fdb_db_t)db)->lock) ((fdb_db_t)db)->lock((fdb_db_t)db); \
} while(0)

// C: 含 return 的 do-while(0) 宏（危险！）
#define _FDB_WRITE_STATUS(db, addr, status_table, status_num, status_index, sync) \
    do { \
        fdb_err_t result = _fdb_write_status(db, addr, status_table, status_num, status_index, sync); \
        if (result != FDB_NO_ERR) return result; \
    } while(0)

#define FLASH_WRITE(db, addr, buf, size, sync) \
    do { \
        fdb_err_t result = _fdb_flash_write(db, addr, buf, size, sync); \
        if (result != FDB_NO_ERR) return result; \
    } while(0)
```

`return` 在宏内展开后从**调用函数**返回，而非宏本身——这是 C 宏的隐式控制流，Rust 中不存在。

## 解决方案

### 场景 A：无 return 的 do-while(0) → 普通方法

```rust
// C: #define db_lock(db) do { if (db->lock) db->lock(db); } while(0)
// Rust:
impl FdbKvdb {
    fn db_lock(&self) {
        if let Some(lock) = self.parent.lock {
            // unsafe FFI 调用
            unsafe { lock(self.parent_mut_ptr()) };
        }
    }
    fn db_unlock(&self) {
        if let Some(unlock) = self.parent.unlock {
            unsafe { unlock(self.parent_mut_ptr()) };
        }
    }
}
```

### 场景 B：含 return 的 do-while(0) → 函数 + `?` 操作符

```rust
// C: _FDB_WRITE_STATUS 宏内含 return result
// Rust: 翻译为函数，return 变为 ?

impl FdbTsdb {
    /// 写入状态（原 _FDB_WRITE_STATUS 宏）
    fn write_status(
        &mut self,
        addr: u32,
        status_table: &mut [u8],
        status_num: usize,
        status_index: usize,
        sync: bool,
    ) -> Result<(), FdbErr> {
        // 原宏内的 "if (result != FDB_NO_ERR) return result" 变为 ?
        self._fdb_write_status(addr, status_table, status_num, status_index, sync)?;
        Ok(())
    }

    /// Flash 写入（原 FLASH_WRITE 宏）
    fn flash_write(
        &mut self,
        addr: u32,
        buf: &[u8],
        sync: bool,
    ) -> Result<(), FdbErr> {
        self._fdb_flash_write(addr, buf, sync)?;
        Ok(())
    }
}

// 调用处：C 宏展开的 return 变为 ? 自动传播
impl FdbTsdb {
    fn write_tsl(&mut self, blob: &FdbBlob) -> Result<(), FdbErr> {
        // C: _FDB_WRITE_STATUS(db, addr, ...);
        // Rust:
        self.write_status(addr, &mut status_table, status_num, status_index, false)?;

        // C: FLASH_WRITE(db, addr, buf, size, sync);
        // Rust:
        self.flash_write(addr, &buf, false)?;

        Ok(())
    }
}
```

### 场景 C：访问器宏 → 方法

```c
// C: 访问器宏
#define db_name(db)      (((fdb_db_t)db)->name)
#define db_sec_size(db)  (((fdb_db_t)db)->sec_size)
#define db_max_size(db)  (((fdb_db_t)db)->max_size)
```

```rust
// Rust: 直接方法
impl FdbKvdb {
    pub fn name(&self) -> &str { self.parent.name }
    pub fn sec_size(&self) -> u32 { self.parent.sec_size }
    pub fn max_size(&self) -> u32 { self.parent.max_size }
}
```

### 场景 D：对齐计算宏 → `const fn`

```c
// C: 对齐计算宏
#define FDB_ALIGN(size, align)  (((size)+(align)-1) - (((size)+(align)-1) % (align)))
#define FDB_WG_ALIGN(size)      (FDB_ALIGN(size, ((FDB_WRITE_GRAN + 7)/8)))
```

```rust
// Rust: const fn 替代宏
pub const fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) / align * align
}

pub const fn wg_align<const GRAN: u32>(size: usize) -> usize {
    let align = (GRAN as usize + 7) / 8;
    align_up(size, align)
}
```

### 场景 E：日志宏 → `log` crate 或 `defmt`

```c
// C: 日志宏
#define FDB_INFO(...)  FDB_LOG_PREFIX(); FDB_PRINT(__VA_ARGS__)
#define FDB_DEBUG(...) FDB_LOG_PREFIX(); FDB_PRINT("(%s:%d) ", __FILE__, __LINE__); FDB_PRINT(__VA_ARGS__)
```

```rust
// Rust: 用 log crate
use log::{info, debug};

// 或用 defmt（嵌入式 no_std）
use defmt::{info, debug};

// 使用
info!("Sector formatted at addr 0x{:08X}", addr);
debug!("KV status: {:?}", kv.status);
```

## ❌ 禁止模式

```rust
// 禁止：用 macro_rules! 翻译含 return 的宏
macro_rules! write_status {
    ($self:expr, $addr:expr, ...) => {{
        let result = $self._fdb_write_status($addr, ...);
        if result.is_err() { return result; }  // 隐式 return——反模式
    }};
}
// 原因：macro 中的 return 难以追踪，Rust 应用函数 + ?

// 禁止：用闭包模拟 do-while(0)
(|| {
    let result = do_something();
    if result.is_err() { return result; }  // 闭包内的 return
    Ok(())
})()
// 原因：可读性差，优先用函数 + ?
```

## 注意事项

- 含 `return` 的 do-while(0) 宏是最危险的——翻译时必须改为独立函数
- `?` 操作符完美替代 `if (err) return err` 模式
- 访问器宏直接翻译为方法，不需要宏
- 对齐计算用 `const fn`，编译时求值，零运行时开销
- 日志宏用 `log`/`defmt` crate，比 C 宏更灵活（支持级别过滤）
