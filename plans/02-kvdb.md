# Plan 2: KVDB — 键值数据库全功能实现

> **总目标**：见 `00-start.md` — FlashDB C→Rust 1:1 翻译迁移
>
> **本 Plan 目标**：翻译 `fdb_kvdb.c`（1944 行 C）为 `src/kvdb.rs`，实现 KVDB 全功能：on-flash header、CRUD、迭代器、GC、recovery、set_default、init/deinit/control。配套完整 UT。
>
> **可与 Plan 3 (TSDB) 并发执行** — 两者互不依赖，仅需 Plan 1 (Foundation) 完成。建议使用独立 git worktree。

---

## 1. IN / OUT Scope

### IN Scope
- `src/kvdb.rs` — KVDB 完整实现（`fdb_kvdb.c` 全部 1944 行翻译）
- KVDB 模块 UT（`cargo test --lib kvdb`）

### OUT Scope
- ❌ BDD Step Definition（Final Plan T20-T21 实现）
- ❌ 修改 Foundation 模块（`def.rs`, `low_lvl.rs`, `flash_trait.rs`, `mock_flash.rs`）— 只引用不修改
- ❌ TSDB 实现（Plan 3）
- ❌ KV 缓存具体策略变更（保持 C 版语义）

---

## 2. 依赖契约

### 2.1 上游依赖（来自 Plan 1，不得修改）

引用 `00-start.md` §4.2 中 Plan 1 提供的公共 API。关键依赖：

| Foundation API | 本 Plan 使用场景 |
|----------------|-----------------|
| `FdbKvdb`, `FdbDb`, `FdbKv`, `FdbBlob`, `FdbErr` (def.rs) | KVDB struct 已在 Plan 1 定义 |
| `FlashDevice` trait (flash_trait.rs) | 所有 flash 操作通过此 trait |
| `MockFlash` (mock_flash.rs) | UT 使用 |
| `set_status/get_status/write_status/read_status` (low_lvl.rs) | sector/kv 状态表操作 |
| `flash_read/flash_erase/flash_write/flash_write_align` (low_lvl.rs) | flash I/O |
| `calc_crc32` (low_lvl.rs 或 crc32.rs) | KV CRC32 校验 |
| `blob_make/blob_read` (low_lvl.rs) | blob 数据传输 |
| `align_up/wg_align/wg_align_down/status_table_size` (low_lvl.rs) | 对齐计算 |
| `init_ex/init_finish/deinit` (init.rs) | KVDB init 复用 Foundation init |

### 2.2 下游契约（提供给 Final Plan）

Final Plan 的 BDD step definition（T20-T21）依赖本 Plan 提供的公共 API：

```rust
// 必须公开的 API（pub，通过 lib.rs 重导出）
impl FdbKvdb {
    pub fn init(...) -> Result<(), FdbErr>;
    pub fn deinit(&mut self) -> Result<(), FdbErr>;
    pub fn control(&mut self, cmd: KvdbControl) -> Result<(), FdbErr>;  // Builder 模式
    pub fn check(&mut self) -> Result<(), FdbErr>;
    pub fn kv_set(&mut self, key: &str, value: &str) -> Result<(), FdbErr>;
    pub fn kv_set_blob(&mut self, key: &str, blob: &mut Blob) -> Result<(), FdbErr>;
    pub fn kv_get(&self, key: &str) -> Option<String>;
    pub fn kv_get_blob(&self, key: &str, blob: &mut Blob) -> Option<usize>;
    pub fn kv_get_obj(&self, key: &str, blob: &mut Blob) -> Option<usize>;
    pub fn kv_del(&mut self, key: &str) -> Result<(), FdbErr>;
    pub fn kv_set_default(&mut self, default_kv: &[DefaultKv]) -> Result<(), FdbErr>;
    pub fn kv_iterator_init(&self) -> KvIterator;
    pub fn kv_iterate(&mut self, iter: &mut KvIterator, cb: impl FnMut(&FdbKv) -> bool) -> bool;
}
```

---

## 3. 涉及目录/模块

```
src/
└── kvdb.rs          # KVDB 完整实现 (fdb_kvdb.c 翻译)
                       — 修改 src/lib.rs 的 mod 声明行（与 Plan 3 协调）
```

---

## 4. 任务清单（T10-T13）

> 本 Plan 内部严格串行：T10 → T11 → T12 → T13。

### T10. kvdb.rs — KV sector header + KV header on-flash struct 翻译 + UT

**What to do**:
- 翻译 `fdb_kvdb.c:102-133` 中的 on-flash struct：
  - `struct sector_hdr_data` → `#[repr(C)] struct SectorHdrData` — 包含 status_table.store, status_table.dirty, magic, combined, reserved, padding
  - `struct kv_hdr_data` → `#[repr(C)] struct KvHdrData` — 包含 status_table, magic, len, crc32, name_len, value_len, padding
- 翻译 `fdb_kvdb.c:34-99` 中的 KVDB 内部常量和宏：
  - `SECTOR_MAGIC_WORD=0x30424446`
  - `KV_MAGIC_WORD=0x3030564B`
  - `GC_MIN_EMPTY_SEC_NUM=1`
  - `FDB_SEC_REMAIN_THRESHOLD`
  - `SECTOR_STORE_OFFSET` / `SECTOR_DIRTY_OFFSET` → `core::mem::offset_of!`
  - `KV_MAGIC_OFFSET` / `KV_LEN_OFFSET` 等 → `offset_of!`
  - `db_name` / `db_init_ok` / `db_sec_size` 等访问器宏 → Rust 方法
- 添加 `const _: () = assert!(core::mem::size_of::<SectorHdrData>() == EXPECTED_SIZE)` 验证布局
- 添加 `const _: () = assert!(core::mem::size_of::<KvHdrData>() == EXPECTED_SIZE)` 验证布局
- 添加 UT：验证 SECTOR_STORE_OFFSET/KV_MAGIC_OFFSET 等与 C 版一致
- 每个结构体添加 `// c: fdb_kvdb.c:LINE` 注释
- 翻译 `fdb_kvdb.c:255-275` 中 KV 缓存操作（如启用 kv_cache feature）：
  - `update_sector_cache` / `get_sector_from_cache` / `update_kv_cache` / `get_kv_from_cache`

**Must NOT do**:
- 不要用 `transmute` 读/写 on-flash struct
- 不要添加 `PhantomData` 到 on-flash struct
- 不要将多个 struct 合并（SectorHdrData 和 KvHdrData 分开定义）
- 不要改变 on-flash 字节布局

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: NO（本 Plan 内部串行）
- Blocks: T11, T12
- Blocked By: Foundation Plan 完成（T2, T4, T5, T6）

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:34-133` — on-flash struct + 常量完整定义
- `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:150-275` — KV 缓存操作函数

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过（layout assertions 不触发）
- [ ] `cargo test --lib kvdb::tests::test_header_layouts` 通过
- [ ] offset_of 结果与 C 版 `offsetof` 一致

**QA Scenarios**:
```
Scenario: on-flash struct 布局验证
  Tool: Bash
  Steps:
    1. cargo test --lib kvdb::tests::test_sector_hdr_size
    2. cargo test --lib kvdb::tests::test_kv_hdr_size
    3. cargo test --lib kvdb::tests::test_offsets — 验证 STORE_OFFSET, KV_MAGIC_OFFSET 等
  Expected Result: 所有 size 和 offset 与 C 版完全一致
```

**Commit**: YES
- Message: `feat(kvdb): on-flash header structs (sector_hdr + kv_hdr) translated with layout verification`

---

### T11. KVDB 核心功能 1: read_kv, find_kv, get_kv 翻译 + UT

**What to do**:
- 翻译 `fdb_kvdb.c:280-346` `find_next_kv_addr` — 按 magic word 扫描下一个 KV
- 翻译 `fdb_kvdb.c:312-346` `get_next_kv_addr` — 获取扇区内下一个 KV 地址
- 翻译 `fdb_kvdb.c:348-414` `read_kv` — 读取 KV 节点（header + name + value + CRC 验证）
- 翻译 `fdb_kvdb.c:416-502` `read_sector_info` — 读取扇区信息（header + 遍历 KV 计算 remain）
- 翻译 `fdb_kvdb.c:504-526` `get_next_sector_addr` — 获取下一个扇区地址
- 翻译 `fdb_kvdb.c:528-557` `kv_iterator` — 通用 KV 迭代器（带回调）
- 翻译 `fdb_kvdb.c:559-607` `find_kv_cb` / `find_kv_no_cache` / `find_kv` — KV 查找（含缓存）
- 翻译 `fdb_kvdb.c:609-644` `fdb_is_str` / `get_kv` — KV 读取
- `void*` 回调参数 → 泛型闭包 `FnMut`
- 每个函数添加 `// c: fdb_kvdb.c:LINE` 注释
- 添加 UT：构造 MockFlash 写入 KV 数据，验证 read_kv 能正确读取并 CRC 校验通过

**Must NOT do**:
- 不要将所有函数塞入主 struct impl
- 不要简化 CRC 计算逻辑
- 不要忽略 FDB_BIG_ENDIAN 条件（当前默认 Little Endian）

**Recommended Agent Profile**:
- Category: `deep`

**Parallelization**:
- Can Run In Parallel: NO（依赖 T10）
- Blocks: T12, T13
- Blocked By: T10

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:280-644` — KV 读取相关完整函数

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib kvdb::tests::test_read_kv` — 读取一个 CRC 正确的 KV
- [ ] `cargo test --lib kvdb::tests::test_find_next_kv_addr` — 扫描到正确的下一个 KV
- [ ] `cargo test --lib kvdb::tests::test_kv_iterator` — 遍历出所有有效 KV

**QA Scenarios**:
```
Scenario: read_kv 读取已写入的 KV
  Tool: Bash
  Steps:
    1. 在 MockFlash 中手工写入一个完整的 KV（sector hdr + kv hdr + name + value + CRC32）
    2. 调用 read_kv(flash, &mut kv)
    3. 验证 kv.crc_is_ok == true, kv.status == WRITE, kv.name == "test", kv.value_len == 4
  Expected Result: read_kv 成功解析 KV 所有字段
```

**Commit**: NO（与 T13 末合并提交）

---

### T12. KVDB 核心功能 2: write_kv, set_kv, del_kv, format_sector, update_sec_status 翻译 + UT

**What to do**:
- 翻译 `fdb_kvdb.c:755-861` — 写入相关函数：
  - `write_kv_hdr` / `format_sector` / `update_sec_status`
- 翻译 `fdb_kvdb.c:864-938` — 分配与扇区迭代：
  - `sector_iterator` / `alloc_kv_cb` / `alloc_kv` / `new_kv` / `new_kv_ex`
- 翻译 `fdb_kvdb.c:940-1096` — 删除/移动/新建：
  - `del_kv` / `move_kv` / `create_kv_blob` / `set_kv`
- 翻译公共 API：
  - `fdb_kv_set` / `fdb_kv_set_blob` / `fdb_kv_get` / `fdb_kv_get_blob` / `fdb_kv_get_obj` / `fdb_kv_to_blob` / `fdb_kv_del`
- `fdb_kv_get` 返回 `Option<String>`（§2 建议项，已确认）
- `fdb_kvdb_control` 的 SET_LOCK/SET_UNLOCK → trait bound 替代
- 每个函数添加 `// c: fdb_kvdb.c:LINE` 注释
- 添加 UT：完整 CRUD cycle（set→get→update→get→del→get→verify deleted）

**Must NOT do**:
- 不要在 on-flash struct 上添加 Rust 字段
- 不要简化两阶段写入（PRE_WRITE → WRITE）逻辑
- 不要合并 fdb_kv_set 和 fdb_kv_set_blob 为一个函数

**Recommended Agent Profile**:
- Category: `deep`

**Parallelization**:
- Can Run In Parallel: NO（依赖 T11）
- Blocks: T13, T18 (Final)
- Blocked By: T11

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:755-1430` — KV 写入+删除+移动完整实现

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib kvdb::tests::test_kv_crud_cycle` — set→get→update→del 完整循环
- [ ] `cargo test --lib kvdb::tests::test_two_phase_write` — 验证 PRE_WRITE→WRITE 状态转换

**QA Scenarios**:
```
Scenario: 完整 CRUD cycle
  Tool: Bash
  Steps:
    1. kv_init(flash) → 成功
    2. kv_set("key1", "value1") → NoErr
    3. kv_get("key1") → Some("value1")
    4. kv_set("key1", "value2") → NoErr
    5. kv_get("key1") → Some("value2")
    6. kv_del("key1") → NoErr
    7. kv_get("key1") → None
  Expected Result: CRUD 循环完整通过，所有返回值正确

Scenario: blob set/get
  Tool: Bash
  Steps:
    1. kv_set_blob("bin", [0x01..0x20], 32) → NoErr
    2. kv_get_blob("bin") → 返回 32 bytes，内容与写入一致
  Expected Result: blob 读写正确
```

**Commit**: NO（与 T13 末合并提交）

---

### T13. KVDB 核心功能 3: gc, iterator, set_default, print, init, check 翻译 + UT

**What to do**:
- 翻译 `fdb_kvdb.c:1098-1181` — GC 实现：
  - `gc_check_cb` / `do_gc` / `gc_collect_by_free_size` / `gc_collect`
- 翻译 `fdb_kvdb.c:1386-1431` — `fdb_kv_set_default`
- 翻译 `fdb_kvdb.c:1432-1507` — `fdb_kv_print`（输出日志可简化为 `log::info!`）
- 翻译 `fdb_kvdb.c:1509-1544` — `kv_auto_update`（如启用 kv_auto_update feature）
- 翻译 `fdb_kvdb.c:1546-1663` — `_fdb_kv_load` 初始化加载（recovery 逻辑）
- 翻译 `fdb_kvdb.c:1672-1727` — `fdb_kvdb_control`（改为 Builder 模式）
- 翻译 `fdb_kvdb.c:1740-1814` — `fdb_kvdb_init` 公共 API
- 翻译 `fdb_kvdb.c:1823-1896` — `fdb_kvdb_deinit` / `fdb_kv_iterator_init` / `fdb_kv_iterate`
- 翻译 `fdb_kvdb.c:1905-1942` — `fdb_kvdb_check`
- `goto __retry` → `loop { break }` 模式
- `goto __exit` → `?` + RAII
- 每个函数添加 `// c: fdb_kvdb.c:LINE` 注释
- 添加 UT：GC 回收已删除 KV 的空间；set_default 恢复默认值；掉电恢复（PRE_WRITE→ERR_HDR, PRE_DELETE→recovery）
- **C 测试等价性迁移**：将原 C 项目 `tests/` 中 KVDB 相关测试数据（CRUD 往返数据、GC 触发条件、recovery 场景、迭代顺序）迁移为 `tests/c-port/kvdb_equiv.rs` integration test，对照 C 版输出验证行为等价性。这些测试数据是 C 作者验证过的金标准，用于捕获翻译错误

**Must NOT do**:
- 不要简化 GC 触发逻辑（空间耗尽 + alloc_kv 失败两种场景）
- 不要忽略 recovery_check 标志
- 不要改变 on-flash 格式

**Recommended Agent Profile**:
- Category: `deep`

**Parallelization**:
- Can Run In Parallel: NO（依赖 T12）
- Blocks: Final Plan T20, T21
- Blocked By: T12

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:1098-1944` — GC + iterator + init + check 完整实现

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib kvdb::tests::test_gc_collect` — GC 回收后有效 KV 仍可找到
- [ ] `cargo test --lib kvdb::tests::test_set_default` — 重置后默认 KV 可读取
- [ ] `cargo test --lib kvdb::tests::test_recovery_pre_write` — PRE_WRITE 状态 KV 被标记 ERR_HDR
- [ ] `cargo test --lib kvdb::tests::test_recovery_pre_delete` — PRE_DELETE 状态 KV 被恢复

**QA Scenarios**:
```
Scenario: GC 回收已删除 KV 空间
  Tool: Bash
  Steps:
    1. 写入 3 个 KV，删除其中 2 个
    2. 继续写入触发 GC
    3. GC 完成后，验证有效 KV 仍可 get
    4. 验证已删除 KV 不可 get
  Expected Result: GC 回收空间，有效 KV 保留

Scenario: set_default 恢复
  Tool: Bash
  Steps:
    1. 写入多个自定义 KV
    2. kv_set_default(default_kvs)
    3. 验证所有自定义 KV 不可 get
    4. 验证所有默认 KV 可 get
  Expected Result: set_default 后只有默认 KV 存在
```

**Commit**: YES
- Message: `feat(kvdb): KVDB 全功能实现完成（init, CRUD, iterator, GC, recovery, set_default）+ UT`

---

## 5. UT 实现要求（§3.4 子 Plan 职责）

本 Plan 完成所有 KVDB 模块 UT，每个 UT 必须：
- 使用 `MockFlash`（来自 Foundation）作为 Flash 后端
- 每个 `assert!` 必须有真实业务逻辑验证（禁止 `assert!(true)`）
- 覆盖正常路径 + 错误路径 + 边界 case（空 KV、超大 value、扇区满、GC 触发）

**函数级覆盖矩阵**：每个公共函数至少 3 个 case（normal / error / boundary）。完整覆盖矩阵见 Final Plan F4 审计引用的覆盖率矩阵，本 Plan 编码时按该矩阵补齐。

**UT 清单**：
| 测试函数 | 覆盖 Task | 验证内容 |
|---------|----------|---------|
| `kvdb::tests::test_header_layouts` | T10 | on-flash struct size + offset |
| `kvdb::tests::test_sector_hdr_size` | T10 | SectorHdrData 大小 |
| `kvdb::tests::test_kv_hdr_size` | T10 | KvHdrData 大小 |
| `kvdb::tests::test_offsets` | T10 | SECTOR_STORE_OFFSET, KV_MAGIC_OFFSET 等 |
| `kvdb::tests::test_read_kv` | T11 | 读取 CRC 正确的 KV |
| `kvdb::tests::test_find_next_kv_addr` | T11 | 扫描下一个 KV 地址 |
| `kvdb::tests::test_kv_iterator` | T11 | 遍历所有有效 KV |
| `kvdb::tests::test_kv_crud_cycle` | T12 | set→get→update→del 循环 |
| `kvdb::tests::test_two_phase_write` | T12 | PRE_WRITE→WRITE 状态转换 |
| `kvdb::tests::test_blob_set_get` | T12 | blob 读写往返 |
| `kvdb::tests::test_gc_collect` | T13 | GC 回收 + 有效 KV 保留 |
| `kvdb::tests::test_set_default` | T13 | set_default 恢复默认 |
| `kvdb::tests::test_recovery_pre_write` | T13 | PRE_WRITE 恢复为 ERR_HDR |
| `kvdb::tests::test_recovery_pre_delete` | T13 | PRE_DELETE 恢复 |

---

## 6. 出口条件（Plan 2 完成判定）

- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib kvdb` 全部 UT 通过
- [ ] `fdb_kvdb.c` 所有公共函数（`fdb_kv_*`, `fdb_kvdb_*`）有对应 Rust pub fn（1:1 映射）
- [ ] 所有 on-flash struct（SectorHdrData, KvHdrData）size_of 编译期断言通过
- [ ] §2.2 下游契约中列出的公共 API 全部实现并可被 Final Plan 引用
- [ ] `cargo test --test c-port::kvdb_equiv` 通过（C 等价性验证）
- [ ] 两次 Commit 完成（T10 后 + T13 后）
