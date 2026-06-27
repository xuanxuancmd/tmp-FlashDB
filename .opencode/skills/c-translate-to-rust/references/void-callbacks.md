# void* 回调参数 → 泛型闭包 / trait

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码大量使用 `void *arg1, void *arg2` 实现类型擦除回调：

```c
// C: void* 回调模式
typedef bool (*fdb_tsl_cb)(fdb_tsl_t tsl, void *arg);

// 迭代器接受回调 + void* 参数
void fdb_tsl_iter(fdb_tsdb_t db, fdb_tsl_cb cb, void *arg) {
    // ... 遍历 ...
    if (cb(&tsl, arg)) {
        break;  // 回调返回 true 停止迭代
    }
}

// 回调实现：void* 强转回具体类型
static bool query_cb(fdb_tsl_t tsl, void *arg) {
    size_t *count = (size_t *)arg;  // void* 强转
    if (tsl->status == FDB_TSL_WRITE) {
        (*count)++;
    }
    return false;  // false = 继续迭代
}

// 使用
size_t count = 0;
fdb_tsl_iter(db, query_cb, &count);
```

## 解决方案

### 方案 A：泛型闭包（推荐，零开销）

```rust
// Rust: 泛型闭包替代 void* 回调
impl FdbTsdb {
    fn tsl_iter<F>(&self, mut cb: F)
    where
        F: FnMut(&FdbTsl) -> bool,  // true = 停止迭代
    {
        // ... 遍历 ...
        for tsl in self.tsls() {
            if cb(&tsl) {
                break;
            }
        }
    }
}

// 使用：闭包自动捕获上下文，无需 void*
let mut count: usize = 0;
db.tsl_iter(|tsl| {
    if tsl.status == TslStatus::Write {
        count += 1;
    }
    false  // 继续迭代
});
```

### 方案 B：`FnMut` + 可变借用（需要修改 tsl）

```rust
impl FdbTsdb {
    fn tsl_iter_mut<F>(&mut self, mut cb: F)
    where
        F: FnMut(&mut FdbTsl) -> bool,
    {
        for tsl in self.tsls_mut() {
            if cb(tsl) {
                break;
            }
        }
    }
}

// 使用：修改 tsl 状态
db.tsl_iter_mut(|tsl| {
    tsl.status = TslStatus::UserStatus1;
    false
});
```

### 方案 C：`Iterator` trait（惯用 Rust 模式）

```rust
// 把迭代器实现为 Iterator trait
struct TslIterator<'a> {
    db: &'a FdbTsdb,
    current: Option<FdbTsl>,
}

impl<'a> Iterator for TslIterator<'a> {
    type Item = FdbTsl;

    fn next(&mut self) -> Option<Self::Item> {
        // ... 获取下一个 TSL ...
        self.current.take()
    }
}

impl FdbTsdb {
    fn tsl_iter(&self) -> TslIterator<'_> {
        TslIterator { db: self, current: None }
    }
}

// 使用：for 循环
let mut count = 0;
for tsl in db.tsl_iter() {
    if tsl.status == TslStatus::Write {
        count += 1;
    }
}
```

### 方案 D：多参数回调（arg1 + arg2 模式）

C 回调经常有两个 void* 参数：

```c
// C: 双 void* 参数
static void kv_iterator(fdb_kvdb_t db, fdb_kv_t kv, void *arg1, void *arg2,
    bool (*callback)(fdb_kv_t kv, void *arg1, void *arg2));

// 回调实现
static bool find_kv_cb(fdb_kv_t kv, void *arg1, void *arg2) {
    const char *key = arg1;      // arg1 = 搜索 key
    bool *find_ok = arg2;        // arg2 = 结果标志
    if (strncmp(kv->name, key, kv->name_len) == 0) {
        *find_ok = true;
        return true;  // 停止迭代
    }
    return false;
}
```

Rust 用闭包捕获替代：

```rust
impl FdbKvdb {
    fn kv_iterator<F>(&self, mut callback: F)
    where
        F: FnMut(&FdbKv) -> bool,
    {
        // ... 遍历 ...
    }
}

// 使用：闭包捕获 key 和 find_ok
fn find_kv(db: &FdbKvdb, key: &str) -> Option<FdbKv> {
    let mut found_kv = None;
    db.kv_iterator(|kv| {
        if kv.name == key {
            found_kv = Some(kv.clone());
            true  // 停止迭代
        } else {
            false
        }
    });
    found_kv
}
```

### 方案 E：`dyn FnMut` trait 对象（动态分派）

当回调需要存储或跨边界传递时：

```rust
// 存储 trait 对象
struct TslIterator<'a> {
    db: &'a FdbTsdb,
    callback: Box<dyn FnMut(&FdbTsl) -> bool>,
}

impl<'a> TslIterator<'a> {
    fn new<F>(db: &'a FdbTsdb, cb: F) -> Self
    where
        F: FnMut(&FdbTsl) -> bool + 'static,
    {
        Self {
            db,
            callback: Box::new(cb),
        }
    }
}
```

## 借用冲突处理

C 的回调中可以修改 db（如 GC 回调 `do_gc` 修改 db），Rust 中 `&self` 借用与 `&mut self` 冲突：

```rust
// 问题：迭代器持有 &self，但回调需要 &mut self（GC 场景）
impl FdbKvdb {
    fn sector_iterator_gc(&mut self) {
        // 方案 1：先收集需要 GC 的扇区信息，再统一处理
        let gc_candidates: Vec<u32> = {
            let mut candidates = Vec::new();
            self.sector_iterator(|sector| {
                if sector.status.dirty == DirtyStatus::True {
                    candidates.push(sector.addr);
                }
                false
            });
            candidates
        };

        // 方案 2：分离状态到独立字段，避免 &mut self 借用冲突
        for addr in gc_candidates {
            self.do_gc(addr);
        }
    }
}
```

## ❌ 禁止模式

```rust
// 禁止：用 *mut c_void 保持 void* 模式
fn tsl_iter(&self, cb: extern "C" fn(*mut FdbTsl, *mut c_void) -> bool, arg: *mut c_void);

// 禁止：用 Any 替代泛型（性能差且类型不安全）
fn tsl_iter(&self, cb: &dyn Fn(&FdbTsl, &dyn Any) -> bool, arg: &dyn Any);
```

## 注意事项

- 闭包捕获是零成本的——编译时单态化，无运行时开销
- `FnMut` 允许修改捕获的变量（如 count++），`Fn` 不允许
- `FnOnce` 消费捕获的变量，适合一次性回调
- GC + 分配递归（alloc_kv → gc_collect → move_kv → alloc_kv）需要重构所有权——先收集再操作
- `Iterator` trait 是最惯用的 Rust 模式，但需要额外实现 `next()` 逻辑
