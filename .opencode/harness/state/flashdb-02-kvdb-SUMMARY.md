# Plan 02-kvdb 编码完成

## 完成情况

| task_id | 描述 | commit | 状态 |
|---------|------|--------|------|
| t-10 | on-flash header structs (SectorHdrData + KvHdrData) + 常量/偏移/KV 缓存操作 + 布局 UT | 1839072 | ✅ 完成 |
| t-11 | read_kv, find_next_kv_addr, get_next_kv_addr, read_sector_info, get_next_sector_addr, kv_iterator, find_kv, get_kv, fdb_is_str + UT | 714df7a | ✅ 完成 |
| t-12 | write_kv_hdr, format_sector, update_sec_status, sector_iterator, alloc_kv, del_kv, move_kv, new_kv, new_kv_ex, create_kv_blob, set_kv, GC (do_gc, gc_collect_by_free_size, gc_collect), 公共 CRUD API (kv_set/kv_set_blob/kv_get/kv_get_blob/kv_get_obj/kv_to_blob/kv_del) + UT | 714df7a | ✅ 完成 |
| t-13 | _fdb_kv_load (recovery), kv_set_default, kv_print, kv_auto_update(cfg), fdb_kvdb_control→builder setters, kvdb_init, kvdb_deinit, kv_iterator_init, kv_iterate, kvdb_check + UT + C 等价性测试 | 714df7a | ✅ 完成 |

## 构建/测试自检

- `cargo check`: pass（默认 no_std + alloc 构建）
- `cargo check --features "kv_auto_update file_mode"`: pass
- `cargo build`: pass
- `cargo test`（全量）: pass
  - lib 单元测试: 86 passed（含 44 个 kvdb:: 模块测试）
  - tests/foundation_equiv.rs: 9 passed
  - tests/c-port/kvdb_equiv.rs (C 等价性): 9 passed
- `cargo test --lib kvdb`: 44 passed

## 实现要点

### 1:1 翻译映射
- `fdb_kvdb.c`（1944 行）→ `src/kvdb.rs`（~3400 行含 UT）
- 所有公共函数（`fdb_kv_*`, `fdb_kvdb_*`）均有对应 Rust `pub fn`，命名 snake_case
- 每个 Rust fn 标注 `// c: fdb_kvdb.c:LINE` 注释

### on-flash 布局兼容
- `SectorHdrData` / `KvHdrData` 使用 `#[repr(C)]`，条件 padding 用 `#[cfg(feature)]` 镜像 C 的 `#if (FDB_WRITE_GRAN == 64/128/256)`
- 6 种 GRAN 配置均有 `const _: () = assert!(size_of == EXPECTED)` 编译期断言
- 偏移量用 `core::mem::offset_of!`（替代 C 的 null 指针 offsetof trick）
- struct flash I/O 通过 `from_bytes`/`to_bytes`（field-by-field LE），无 `transmute`、无 `unsafe`
- CRC32 复用 Foundation `calc_crc32`（与 C 字节级一致）

### 关键模式翻译（对照 skill references）
- `goto __retry` (new_kv) → `loop { continue }`
- `goto __exit` (move_kv, kv_set_default) → `?` + 顺序清理
- `goto __reload` (print_kv_cb) → 收集 value 后直接打印（函数提取）
- `void*` 回调 (`kv_iterator`, `sector_iterator`) → 泛型闭包 `FnMut(&F, &FdbKv) -> bool`
- `fdb_kvdb_control` SET_LOCK/SET_UNLOCK (`void*` 函数指针强转) → builder setter 方法（`set_lock(fn(&mut FdbDb))`）
- C 继承 (`fdb_kvdb { fdb_db parent }`) → 组合 + `AsRef<FdbDb>`（Foundation 已建）

### 借用重构（Rust 所有权适配）
- GC/恢复路径（`do_gc`, `check_sec_hdr`, `check_and_recovery_kv`）的 C 回调内 `&mut db` 与迭代器 `&mut db` 冲突 → 重构为"先收集再操作"或显式循环 + reborrow，行为等价（见 references/void-callbacks.md, goto-patterns.md）
- `read_kv` 可写 ERR_HDR（自愈）→ 读取链函数均取 `&mut F flash`（忠实于 C 的可变 `fdb_kvdb_t`）
- `FdbKvdb` 不持有 flash（Foundation 移除 C `union storage`）→ 所有 flash 操作通过 `&mut F: FlashDevice` 参数传入

### API 签名说明
- 公共方法增加 `flash: &mut F`（或 `&F`）参数：Foundation 用 `FlashDevice` trait 替代 C 的 FAL/file 后端，`FdbKvdb` 不持有 flash，故需显式传入。这是 Foundation 设计的必然结果（Plan §2.2 契约为示意签名）。
- `fdb_kv_get` 返回 `Option<String>`（§2 建议项已确认），需 `extern crate alloc`（已加到 lib.rs）。
- `kv_get_obj` 返回 `bool` + out 参数 `&mut FdbKv`（C 返回 `fdb_kv_t`/NULL 的安全映射）。
- `kv_set_default` 使用 `self.default_kvs`（init 时设置），不另传参数（忠实于 C）。

## 已知限制
- `kv_cache` feature 无法编译：Foundation `def.rs:504` 的 `FdbKvdb::default()` 用 `[KvdbSecInfo::default(); N]` 数组初始化要求 `KvdbSecInfo: Copy`，但该类型仅 `Clone`。此为 Foundation 预存问题，按约束不得修改 def.rs。`kvdb.rs` 内的缓存代码已修正为正确（`get_kv_from_cache` 取 `&mut self` 并直接更新 active）。默认构建（无 kv_cache）完全正常，Plan 出口条件基于默认 features。

## 出口条件核对
- [x] `cargo build` 编译通过
- [x] `cargo test --lib kvdb` 全部 UT 通过（44 测试）
- [x] `fdb_kvdb.c` 所有公共函数有对应 Rust pub fn（1:1 映射）
- [x] 所有 on-flash struct size_of 编译期断言通过
- [x] §2.2 下游契约公共 API 全部实现
- [x] `cargo test --test kvdb_equiv` 通过（9 测试）
- [x] 两次 Commit 完成（T10 后 + T13 后）
