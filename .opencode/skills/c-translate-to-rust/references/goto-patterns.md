# goto 模式 → ? / loop / 状态机

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码用 goto 实现三种控制流模式：

### 模式 1：`goto __exit`（清理后退出，12 处）

```c
// C: goto __exit 清理模式
static fdb_err_t move_kv(fdb_kvdb_t db, fdb_kv_t kv) {
    fdb_err_t result = FDB_NO_ERR;
    // ... 分配新空间 ...
    if (empty_kv == FAILED_ADDR) {
        result = FDB_SAVED_FULL;
        goto __exit;  // 错误退出
    }
    // ... 正常逻辑 ...
__exit:
    // 清理代码
    db->in_recovery_check = false;
    return result;
}
```

### 模式 2：`goto __retry`（重试循环，3 处）

```c
// C: goto __retry 重试模式
static uint32_t new_kv(fdb_kvdb_t db, kv_sec_info_t sector, size_t kv_size) {
    bool already_gc = false;
__retry:
    if ((empty_kv = alloc_kv(db, sector, kv_size)) == FAILED_ADDR) {
        if (db->gc_request && !already_gc) {
            gc_collect_by_free_size(db, kv_size);
            already_gc = true;
            goto __retry;  // GC 后重试
        }
    }
    return empty_kv;
}
```

### 模式 3：`goto __reload`（前向跳转重执行，1 处）

```c
// C: goto __reload 前向跳转
static bool print_kv_cb(fdb_kv_t kv, void *arg1, void *arg2) {
    // ... 读取 KV ...
    if (kv_hdr.name_len > 0) {
        goto __reload;  // 跳到重新加载逻辑
    }
    // ... 其他逻辑 ...
__reload:
    // 重新读取并打印
    _fdb_flash_read(db, kv->addr.value, (uint32_t *)buf, size);
    // ...
}
```

## 解决方案

### 模式 1：`goto __exit` → `?` + RAII Drop

```rust
// Rust: 用 Result + ? 操作符替代 goto __exit
fn move_kv(&mut self, kv: &mut FdbKv) -> Result<(), FdbErr> {
    // 分配新空间，失败时 ? 自动 return Err
    let empty_kv = self.alloc_kv(sector, kv_size)
        .ok_or(FdbErr::SavedFull)?;

    // 正常逻辑——失败自动退出
    self.write_kv_hdr(empty_kv, &kv_hdr)?;
    self.flash.write(empty_kv + KV_HDR_DATA_SIZE, key.as_bytes())?;
    self.flash.write(empty_kv + KV_HDR_DATA_SIZE + key.len(), value)?;

    Ok(())  // 成功返回
}
```

如果需要清理（如释放锁），用 RAII guard：

```rust
struct RecoveryGuard<'a> {
    db: &'a mut FdbKvdb,
}
impl Drop for RecoveryGuard<'_> {
    fn drop(&mut self) {
        self.db.in_recovery_check = false;
    }
}

fn move_kv(&mut self, kv: &mut FdbKv) -> Result<(), FdbErr> {
    let _guard = RecoveryGuard { db: self };
    self.db.in_recovery_check = true;

    let empty_kv = self.alloc_kv(sector, kv_size)
        .ok_or(FdbErr::SavedFull)?;
    // ... 正常逻辑 ...
    Ok(())  // _guard drop → in_recovery_check = false
}
```

### 模式 2：`goto __retry` → `loop { break }`

```rust
// Rust: 用 loop + break 替代 goto __retry
fn new_kv(&mut self, sector: &mut KvdbSecInfo, kv_size: usize) -> u32 {
    let mut already_gc = false;
    loop {
        if let Some(empty_kv) = self.alloc_kv(sector, kv_size) {
            return empty_kv;  // break + return
        }
        if self.gc_request && !already_gc {
            self.gc_collect_by_free_size(kv_size);
            already_gc = true;
            continue;  // goto __retry
        }
        return FAILED_ADDR;  // GC 后仍失败
    }
}
```

### 模式 3：`goto __reload` → 状态机或函数提取

```rust
// Rust: 重构为显式状态或提取函数
fn print_kv_cb(&mut self, kv: &mut FdbKv) -> bool {
    // ... 读取 KV header ...

    if kv_hdr.name_len > 0 {
        // 直接调用 reload 逻辑，而非 goto
        self.reload_and_print(kv, &kv_hdr);
        return false;
    }
    // ... 其他逻辑 ...
    false
}

fn reload_and_print(&mut self, kv: &FdbKv, hdr: &KvHdrData) {
    let mut buf = [0u8; 32];
    self.flash.read(kv.addr.value, &mut buf).ok();
    // ... 打印 ...
}
```

或者用枚举状态机：

```rust
enum PrintState {
    Initial,
    Reloaded,
}

fn print_kv_cb(&mut self, kv: &mut FdbKv) -> bool {
    let mut state = PrintState::Initial;
    loop {
        match state {
            PrintState::Initial => {
                // ... 读取 KV ...
                if kv_hdr.name_len > 0 {
                    state = PrintState::Reloaded;  // goto __reload
                    continue;
                }
                // ... 其他逻辑 ...
                return false;
            }
            PrintState::Reloaded => {
                // 重新读取并打印
                self.flash.read(kv.addr.value, &mut buf).ok();
                // ...
                return false;
            }
        }
    }
}
```

## do-while(0) 宏含 return 的处理

C 中 `do { ...; if (err) return err; } while(0)` 宏内含 return 语句：

```c
// C: 宏内含 return
#define _FDB_WRITE_STATUS(db, addr, status_table, status_num, status_index, sync) \
    do { \
        fdb_err_t result = _fdb_write_status(db, addr, status_table, status_num, status_index, sync); \
        if (result != FDB_NO_ERR) return result; \
    } while(0)
```

Rust 翻译为函数 + `?` 操作符：

```rust
// Rust: 宏 → 函数，return → ?
fn write_status(
    &mut self,
    addr: u32,
    status_table: &mut [u8],
    status_num: usize,
    status_index: usize,
    sync: bool,
) -> Result<(), FdbErr> {
    // 原宏内的 return result 变为 ?
    self._fdb_write_status(addr, status_table, status_num, status_index, sync)?;
    Ok(())
}

// 调用处：
fn some_function(&mut self) -> Result<(), FdbErr> {
    // C: _FDB_WRITE_STATUS(db, addr, ...);
    // Rust:
    self.write_status(addr, &mut status_table, status_num, status_index, sync)?;
    // ... 后续逻辑 ...
    Ok(())
}
```

## ❌ 禁止模式

```rust
// 禁止：Rust 无 goto，不能直接翻译
'exit: {
    if err {
        break 'exit;  // 类似 goto，但只能向后跳
    }
    // ...
}
// 这虽然合法但可读性差，优先用 ? 和 Result

// 禁止：用 unsafe 模拟 goto
// Rust 根本没有 goto 语法，不要尝试
```

## 注意事项

- `goto __exit` 是最常见的模式（12/16 处），`?` + `Drop` 几乎总能完美替代
- `goto __retry` 用 `loop { continue/break }` 替代，语义清晰
- `goto __reload` 是前向跳转（唯一 1 处），必须重构为函数提取或状态机
- do-while(0) 宏含 `return` 是最危险的——宏展开后 return 从调用函数返回，Rust 中必须改为独立函数 + `?`
