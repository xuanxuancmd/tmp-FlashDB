# void* → 函数指针强转 → 重新设计

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码用 `void*` 传递函数指针，需要 `#pragma GCC diagnostic` 抑制警告：

```c
// C: fdb_kvdb_control 用 void* arg 传递各种类型的值
void fdb_kvdb_control(fdb_kvdb_t db, int cmd, void *arg) {
    switch (cmd) {
    case FDB_KVDB_CTRL_SET_SEC_SIZE:
        db->parent.sec_size = *(uint32_t *)arg;  // void* → uint32_t*
        break;
    case FDB_KVDB_CTRL_GET_SEC_SIZE:
        *(uint32_t *)arg = db->parent.sec_size;
        break;
    case FDB_KVDB_CTRL_SET_LOCK:
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wpedantic"
        db->parent.lock = (void (*)(fdb_db_t db))arg;  // void* → 函数指针
#pragma GCC diagnostic pop
        break;
    case FDB_KVDB_CTRL_SET_FILE_MODE:
        db->parent.file_mode = *(bool *)arg;
        break;
    // ... 更多 case ...
    }
}
```

这是**类型不安全的命令模式**——`void* arg` 可以是任何类型，编译器无法检查。

## 解决方案

### 方案 A：Builder 模式（推荐，替代 control 命令）

```rust
// C: fdb_kvdb_control(db, FDB_KVDB_CTRL_SET_SEC_SIZE, &sec_size)
// Rust: Builder 模式

pub struct KvdbConfig {
    sec_size: Option<u32>,
    max_size: Option<u32>,
    file_mode: bool,
    not_formatable: bool,
    lock: Option<Box<dyn Fn(&mut FdbDb) + Send + Sync>>,
    unlock: Option<Box<dyn Fn(&mut FdbDb) + Send + Sync>>,
}

impl Default for KvdbConfig {
    fn default() -> Self {
        Self {
            sec_size: None,
            max_size: None,
            file_mode: false,
            not_formatable: false,
            lock: None,
            unlock: None,
        }
    }
}

impl KvdbConfig {
    pub fn sec_size(mut self, size: u32) -> Self {
        self.sec_size = Some(size);
        self
    }

    pub fn max_size(mut self, size: u32) -> Self {
        self.max_size = Some(size);
        self
    }

    pub fn file_mode(mut self, enabled: bool) -> Self {
        self.file_mode = enabled;
        self
    }

    pub fn lock<F>(mut self, lock_fn: F) -> Self
    where
        F: Fn(&mut FdbDb) + Send + Sync + 'static,
    {
        self.lock = Some(Box::new(lock_fn));
        self
    }

    pub fn unlock<F>(mut self, unlock_fn: F) -> Self
    where
        F: Fn(&mut FdbDb) + Send + Sync + 'static,
    {
        self.unlock = Some(Box::new(unlock_fn));
        self
    }
}

// 使用：类型安全的配置
let kvdb = FdbKvdb::init("mydb", "fdb_kvdb1", KvdbConfig::default()
    .sec_size(4096)
    .max_size(4096 * 4)
    .file_mode(true)
    .lock(|db| { /* 加锁逻辑 */ })
    .unlock(|db| { /* 解锁逻辑 */ })
)?;
```

### 方案 B：enum 命令（保持 control API 风格）

```rust
// 如果必须保持 control API 风格，用 enum 替代 int cmd + void* arg
pub enum KvdbControl {
    SetSecSize(u32),
    GetSecSize,           // 返回 u32
    SetLock(Box<dyn Fn(&mut FdbDb) + Send + Sync>),
    SetUnlock(Box<dyn Fn(&mut FdbDb) + Send + Sync>),
    SetFileMode(bool),
    SetMaxSize(u32),
    SetNotFormat(bool),
}

impl FdbKvdb {
    pub fn control(&mut self, cmd: KvdbControl) -> Result<u32, FdbErr> {
        match cmd {
            KvdbControl::SetSecSize(size) => {
                self.parent.sec_size = size;
                Ok(0)
            }
            KvdbControl::GetSecSize => {
                Ok(self.parent.sec_size)
            }
            KvdbControl::SetLock(lock_fn) => {
                self.parent.lock = Some(lock_fn);
                Ok(0)
            }
            KvdbControl::SetUnlock(unlock_fn) => {
                self.parent.unlock = Some(unlock_fn);
                Ok(0)
            }
            KvdbControl::SetFileMode(enabled) => {
                self.parent.file_mode = enabled;
                Ok(0)
            }
            // ...
        }
    }
}

// 使用：类型安全，编译器检查参数类型
db.control(KvdbControl::SetSecSize(4096))?;
let sec_size = db.control(KvdbControl::GetSecSize)?;
```

### 方案 C：直接方法调用（最简单）

```rust
// C: fdb_kvdb_control(db, FDB_KVDB_CTRL_SET_SEC_SIZE, &val)
// Rust: 直接方法
impl FdbKvdb {
    pub fn set_sec_size(&mut self, size: u32) -> Result<(), FdbErr> {
        if self.parent.init_ok {
            return Err(FdbErr::InitFailed);  // 必须在 init 前设置
        }
        self.parent.sec_size = size;
        Ok(())
    }

    pub fn sec_size(&self) -> u32 {
        self.parent.sec_size
    }

    pub fn set_lock<F>(&mut self, lock_fn: F)
    where
        F: Fn(&mut FdbDb) + Send + Sync + 'static,
    {
        self.parent.lock = Some(Box::new(lock_fn));
    }
}
```

## ❌ 禁止模式

```rust
// 禁止：保持 C 的 void* control API
fn control(&mut self, cmd: i32, arg: *mut c_void) {
    match cmd {
        0x00 => { self.parent.sec_size = unsafe { *(arg as *const u32) }; }
        0x02 => { self.parent.lock = unsafe { Some(core::mem::transmute(arg)) }; }
        _ => {}
    }
}
// 原因：类型不安全，transmute 是 UB 风险

// 禁止：transmute void* 到函数指针
let lock_fn = unsafe { core::mem::transmute::<*mut c_void, extern "C" fn(*mut FdbDb)>(arg) };
// 原因：transmute 在非 FFI 边界是禁令
```

## #pragma GCC diagnostic 的处理

C 代码用 `#pragma GCC diagnostic push/pop` 抑制 void*→函数指针强转的 `-Wpedantic` 警告：

```c
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wpedantic"
db->parent.lock = (void (*)(fdb_db_t db))arg;
#pragma GCC diagnostic pop
```

Rust 中**直接删除**——重新设计后不再需要 void*→函数指针强转。如果 FFI 边界必须转换，用 `extern "C" fn` 类型：

```rust
// FFI 边界（仅在 extern "C" 函数内部）：
unsafe extern "C" fn control_ffi(db: *mut FdbKvdb, cmd: u32, arg: *mut c_void) -> i32 {
    let db = unsafe { &mut *db };
    match cmd {
        0x00 => {
            db.set_sec_size(unsafe { *arg as u32 });
            0
        }
        // ...
        _ => -1,
    }
}
```

## 注意事项

- Builder 模式是最 Rust 惯用的方式，类型安全且可链式调用
- `#[track_caller]` 可用于 control 方法，提供更好的错误定位
- lock/unlock 用 `Box<dyn Fn>` 而非裸函数指针——闭包可捕获上下文
- 初始化前设置检查（`if self.parent.init_ok`）保留在方法中
