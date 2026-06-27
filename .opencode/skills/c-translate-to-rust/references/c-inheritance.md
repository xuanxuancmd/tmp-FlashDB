# C 继承（struct 嵌入）→ 组合 + AsRef

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 用 struct 嵌入模拟继承，子 struct 通过指针强转访问父字段：

```c
// C: struct 嵌入模拟继承
struct fdb_db {
    const char *name;
    uint32_t sec_size;
    // ...
};

struct fdb_kvdb {
    struct fdb_db parent;  // "继承" fdb_db
    struct fdb_default_kv default_kvs;
    bool gc_request;
    // ...
};

// C: 通过指针强转向上转型
#define db_name(db)  (((fdb_db_t)db)->name)  // fdb_kvdb_t → fdb_db_t
#define db_sec_size(db)  (((fdb_db_t)db)->sec_size)

// 使用：
static void update_sector_cache(fdb_kvdb_t db) {
    // db 是 fdb_kvdb_t，但通过宏访问 fdb_db 的字段
    if (db_name(db)) { ... }
}
```

## 解决方案

### 方案 A：组合 + 显式访问（推荐）

```rust
// 父 struct
#[repr(C)]
struct FdbDb {
    name: &'static str,
    sec_size: u32,
    max_size: u32,
    oldest_addr: u32,
    init_ok: bool,
    // ...
}

// 子 struct 通过组合持有父 struct
struct FdbKvdb {
    parent: FdbDb,  // 组合替代继承
    default_kvs: FdbDefaultKv,
    gc_request: bool,
    // ...
}

// 访问父字段：显式通过 .parent
impl FdbKvdb {
    fn update_sector_cache(&mut self) {
        if !self.parent.name.is_empty() {
            // ...
        }
    }
}
```

### 方案 B：`AsRef<Parent>` trait（提供向上转型）

```rust
impl AsRef<FdbDb> for FdbKvdb {
    fn as_ref(&self) -> &FdbDb {
        &self.parent
    }
}

impl AsMut<FdbDb> for FdbKvdb {
    fn as_mut(&mut self) -> &mut FdbDb {
        &mut self.parent
    }
}

// 使用：需要父类型引用时用 as_ref()
fn log_db_info(db: &impl AsRef<FdbDb>) {
    let parent = db.as_ref();
    println!("DB: {}, sec_size: {}", parent.name, parent.sec_size);
}

// 传入 FdbKvdb：
fn use_kvdb(kvdb: &FdbKvdb) {
    log_db_info(kvdb);  // 自动 AsRef 转换
}
```

### 方案 C：访问器方法替代宏

```rust
// C 宏: #define db_name(db) (((fdb_db_t)db)->name)
// Rust: 直接方法
impl FdbKvdb {
    fn name(&self) -> &str { self.parent.name }
    fn sec_size(&self) -> u32 { self.parent.sec_size }
    fn max_size(&self) -> u32 { self.parent.max_size }
    fn oldest_addr(&self) -> u32 { self.parent.oldest_addr }
}

impl FdbTsdb {
    fn name(&self) -> &str { self.parent.name }
    fn sec_size(&self) -> u32 { self.parent.sec_size }
}
```

### 方案 D：公共 trait（当多个子类型需要相同接口时）

```rust
// 定义公共接口 trait
trait DbOperations {
    fn name(&self) -> &str;
    fn sec_size(&self) -> u32;
    fn max_size(&self) -> u32;
    fn oldest_addr(&self) -> u32;
    fn init_ok(&self) -> bool;
}

// FdbKvdb 实现
impl DbOperations for FdbKvdb {
    fn name(&self) -> &str { self.parent.name }
    fn sec_size(&self) -> u32 { self.parent.sec_size }
    // ...
}

// FdbTsdb 实现
impl DbOperations for FdbTsdb {
    fn name(&self) -> &str { self.parent.name }
    fn sec_size(&self) -> u32 { self.parent.sec_size }
    // ...
}

// 泛型函数接受任何实现 DbOperations 的类型
fn log_db<T: DbOperations>(db: &T) {
    println!("DB: {}, sec_size: {}", db.name(), db.sec_size());
}
```

## db_lock / db_unlock 宏的处理

C 用 `do-while(0)` 宏封装 lock/unlock 并通过指针强转调用父字段：

```c
// C
#define db_lock(db) do { \
    if (((fdb_db_t)db)->lock) ((fdb_db_t)db)->lock((fdb_db_t)db); \
} while(0)
```

Rust 翻译为方法：

```rust
impl FdbKvdb {
    fn db_lock(&self) {
        if let Some(lock) = self.parent.lock {
            lock(&mut self.parent as *mut _);  // unsafe FFI
        }
    }
    fn db_unlock(&self) {
        if let Some(unlock) = self.parent.unlock {
            unlock(&mut self.parent as *mut _);
        }
    }
}
```

## ❌ 禁止模式

```rust
// 禁止：用 Deref 模拟继承（反模式）
impl Deref for FdbKvdb {
    type Target = FdbDb;
    fn deref(&self) -> &FdbDb { &self.parent }
}
// 原因：Deref 隐式转换导致代码不可追踪，且破坏 1:1 映射

// 禁止：指针强转模拟向上转型
let db: &FdbDb = unsafe { &*(kvdb as *const FdbKvdb as *const FdbDb) };
// 原因：依赖内存布局假设，UB 风险
```

## 注意事项

- on-flash struct（如 `sector_hdr_data`）不需要组合模式，它们是独立类型
- `AsRef` 是零成本抽象（编译时单态化），无运行时开销
- 如果父 struct 有条件编译字段（`#[cfg]`），子 struct 的 `AsRef` 实现也需要对应 `#[cfg]`
- FdbKvdb 和 FdbTsdb 不需要 `#[repr(C)]`（它们是内存中的，不写 flash），只有 FdbDb 中的 on-flash 部分（如 storage）需要 `#[repr(C)]`
