# Plan 3: TSDB — 时序数据库全功能实现

> **总目标**：见 `00-start.md` — FlashDB C→Rust 1:1 翻译迁移
>
> **本 Plan 目标**：翻译 `fdb_tsdb.c`（1118 行 C）为 `src/tsdb.rs`，实现 TSDB 全功能：on-flash header、append、iter/iter_reverse/iter_by_time、query_count、set_status、clean、init/deinit/control、recovery。配套完整 UT。
>
> **可与 Plan 2 (KVDB) 并发执行** — 两者互不依赖，仅需 Plan 1 (Foundation) 完成。建议使用独立 git worktree。

---

## 1. IN / OUT Scope

### IN Scope
- `src/tsdb.rs` — TSDB 完整实现（`fdb_tsdb.c` 全部 1118 行翻译）
- TSDB 模块 UT（`cargo test --lib tsdb`）

### OUT Scope
- ❌ BDD Step Definition（Final Plan T22-T24 实现）
- ❌ 修改 Foundation 模块（`def.rs`, `low_lvl.rs`, `flash_trait.rs`, `mock_flash.rs`）— 只引用不修改
- ❌ KVDB 实现（Plan 2）
- ❌ FAL 具体实现（第一阶段不实现）

---

## 2. 依赖契约

### 2.1 上游依赖（来自 Plan 1，不得修改）

引用 `00-start.md` §4.2 中 Plan 1 提供的公共 API。关键依赖：

| Foundation API | 本 Plan 使用场景 |
|----------------|-----------------|
| `FdbTsdb`, `FdbDb`, `FdbTsl`, `FdbBlob`, `FdbErr` (def.rs) | TSDB struct 已在 Plan 1 定义 |
| `FlashDevice` trait (flash_trait.rs) | 所有 flash 操作通过此 trait |
| `MockFlash` (mock_flash.rs) | UT 使用 |
| `set_status/get_status/write_status/read_status` (low_lvl.rs) | sector/tsl 状态表操作 |
| `flash_read/flash_erase/flash_write/flash_write_align` (low_lvl.rs) | flash I/O |
| `calc_crc32` (low_lvl.rs 或 crc32.rs) | TSL CRC32 校验 |
| `blob_make/blob_read` (low_lvl.rs) | blob 数据传输 |
| `align_up/wg_align/wg_align_down/status_table_size` (low_lvl.rs) | 对齐计算 |
| `init_ex/init_finish/deinit` (init.rs) | TSDB init 复用 Foundation init |

### 2.2 下游契约（提供给 Final Plan）

Final Plan 的 BDD step definition（T22-T24）依赖本 Plan 提供的公共 API：

```rust
// 必须公开的 API（pub，通过 lib.rs 重导出）
impl FdbTsdb {
    pub fn init(...) -> Result<(), FdbErr>;
    pub fn deinit(&mut self) -> Result<(), FdbErr>;
    pub fn control(&mut self, cmd: TsdbControl) -> Result<(), FdbErr>;  // Builder 模式
    pub fn tsl_append(&mut self, blob: &Blob) -> Result<(), FdbErr>;
    pub fn tsl_append_with_ts(&mut self, blob: &Blob, ts: fdb_time_t) -> Result<(), FdbErr>;
    pub fn tsl_iter(&self, cb: impl FnMut(&FdbTsl) -> bool) -> bool;
    pub fn tsl_iter_reverse(&self, cb: impl FnMut(&FdbTsl) -> bool) -> bool;
    pub fn tsl_iter_by_time(&self, from: fdb_time_t, to: fdb_time_t, cb: impl FnMut(&FdbTsl) -> bool) -> bool;
    pub fn tsl_query_count(&self, from: fdb_time_t, to: fdb_time_t) -> usize;
    pub fn tsl_max_blob_count(&self, from: fdb_time_t, to: fdb_time_t, blob_len: usize) -> usize;
    pub fn tsl_set_status(&mut self, status: TslStatus, blob: &Blob) -> Result<(), FdbErr>;
    pub fn tsl_to_blob(&self, tsl: &FdbTsl, blob: &mut Blob) -> usize;
    pub fn tsl_clean(&mut self) -> Result<(), FdbErr>;
}
```

---

## 3. 涉及目录/模块

```
src/
└── tsdb.rs          # TSDB 完整实现 (fdb_tsdb.c 翻译)
                       — 修改 src/lib.rs 的 mod 声明行（与 Plan 2 协调）
```

---

## 4. 任务清单（T14-T18）

> 本 Plan 内部严格串行：T14 → T15 → T16 → T17 → T18。

### T14. tsdb.rs — TSDB sector header + log index on-flash struct 翻译 + UT

**What to do**:
- 翻译 `fdb_tsdb.c:28-99` 中 TSDB 内部常量和宏：
  - `SECTOR_MAGIC_WORD=0x304C5354`
  - `TSL_STATUS_TABLE_SIZE` / `TSL_UINT32_ALIGN_SIZE`
  - `TSL_TIME_ALIGN_SIZE`（根据 timestamp_64bit feature）
  - `LOG_IDX_DATA_SIZE` / various offsets → `offset_of!`
  - `SECTOR_MAGIC_OFFSET` / `SECTOR_START_TIME_OFFSET` 等
  - `FDB_TSDB_FIXED_BLOB_SIZE`（如启用）→ Cargo feature
  - `FAILED_ADDR=0xFFFFFFFF`
  - `db_name` / `db_init_ok` / `db_sec_size` / `db_max_size` / `db_oldest_addr` → methods
  - `db_lock` / `db_unlock` → methods（可选 trait bound）
  - `_FDB_WRITE_STATUS` / `FLASH_WRITE` → inline helper fn
- 翻译 `fdb_tsdb.c:101-133` 中 on-flash struct：
  - `struct sector_hdr_data` → `#[repr(C)] struct TsdbSectorHdrData` — 包含 status, magic, start_time, end_info[2]*{time, index, status}, reserved, padding
  - `struct log_idx_data` → `#[repr(C)] struct LogIdxData` — 包含 status_table, time(根据 64bit feature), log_len, log_addr, padding
- 添加 `const _: () = assert!(core::mem::size_of::<TsdbSectorHdrData>() == EXPECTED)` 验证布局
- 添加 `const _: () = assert!(core::mem::size_of::<LogIdxData>() == EXPECTED)` 验证布局
- 添加 UT：验证所有 offset 和 size 与 C 版一致
- 每个结构体添加 `// c: fdb_tsdb.c:LINE` 注释

**Must NOT do**:
- 不要添加 Rust 字段到 on-flash struct
- 不要用 `transmute` 读/写
- 不要将 `sector_hdr` 和 `log_idx` 合并到同一 struct

**Recommended Agent Profile**:
- Category: `quick`

**Parallelization**:
- Can Run In Parallel: NO（本 Plan 内部串行）
- Blocks: T15, T16
- Blocked By: Foundation Plan 完成（T2, T4, T5, T6）

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:28-133` — TSDB on-flash struct + 常量

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过（layout 验证不触发）
- [ ] `cargo test --lib tsdb::tests::test_header_layouts`
- [ ] offset_of 值与 C 版一致

**QA Scenarios**:
```
Scenario: TSDB on-flash struct size 验证
  Tool: Bash
  Steps:
    1. cargo test --lib tsdb::tests::test_tsdb_sector_hdr_size
    2. cargo test --lib tsdb::tests::test_log_idx_size
    3. cargo test --lib tsdb::tests::test_tsdb_offsets
  Expected Result: 所有 size 和 offset 与 C 版一致
```

**Commit**: YES
- Message: `feat(tsdb): on-flash structs (sector_hdr + log_idx) translated with layout verification`

---

### T15. TSDB 核心功能 1: read_tsl, format_sector, write_tsl, update_sec_status 翻译 + UT

**What to do**:
- 翻译 `fdb_tsdb.c:147-175` — `read_tsl` 读取 TSL 节点
- 翻译 `fdb_tsdb.c:177-240` — 扇区/TSL 地址操作：
  - `get_next_sector_addr` / `get_next_tsl_addr` / `get_last_tsl_addr` / `get_last_sector_addr`
- 翻译 `fdb_tsdb.c:242-327` — 扇区读取与格式化：
  - `read_sector_info` / `format_sector`
- 翻译 `fdb_tsdb.c:350-499` — TSL 写入：
  - `write_tsl` / `update_sec_status` / `tsl_append`
- `_FDB_WRITE_STATUS` / `FLASH_WRITE` 宏 → 辅助 fn 返回 `Result`
- `goto __exit` → `?` + RAII
- 公共 API `fdb_tsl_append` / `fdb_tsl_append_with_ts`
- 每个函数添加 `// c: fdb_tsdb.c:LINE` 注释
- 添加 UT：写入 TSL 后 read_tsl 验证数据一致；append 后时间戳递增验证；扇区满后自动切换

**Must NOT do**:
- 不要改变 TSL 索引布局
- 不要忽略 PRE_WRITE → WRITE 两阶段状态转换
- 不要简化 update_sec_status 中的 end_info 保存逻辑

**Recommended Agent Profile**:
- Category: `deep`

**Parallelization**:
- Can Run In Parallel: NO（依赖 T14）
- Blocks: T16, T17
- Blocked By: T14, T7, T8

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:147-499` — TSDB 核心读写扇区操作

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib tsdb::tests::test_append_tsl` — append 后读取 TSL 数据一致
- [ ] `cargo test --lib tsdb::tests::test_timestamp_ordering` — 时间戳必须单调递增

**QA Scenarios**:
```
Scenario: TSL append + read 往返一致
  Tool: Bash
  Steps:
    1. tsdb_init(mock_flash, get_time=|| 1000, max_len=256)
    2. tsl_append_with_ts(64 bytes blob, timestamp=100)
    3. 手动调用 read_tsl 读取返回的 idx
    4. 验证 tsl.time == 1000, tsl.log_len == 64
  Expected Result: TSL 数据与写入一致
```

**Commit**: NO（与 T16 末合并提交）

---

### T16. TSDB 核心功能 2: init, deinit, control, recovery, clean 翻译 + UT

**What to do**:
- 翻译 `fdb_tsdb.c:866-915` — TSDB 初始化辅助：
  - `check_sec_hdr_cb` → fn + 状态收集 struct
  - `format_all_cb` / `tsl_format_all`
- 翻译 `fdb_tsdb.c:924-929` — `fdb_tsl_clean`
- 翻译 `fdb_tsdb.c:938-1003` — `fdb_tsdb_control`（改为 Builder）
- 翻译 `fdb_tsdb.c:1018-1116` — `fdb_tsdb_init` / `fdb_tsdb_deinit`
- control 命令模式 → Builder 模式 + type-safe setter
- lock/unlock → trait bound 替代 `void*` 函数指针
- 每个函数添加 `// c: fdb_tsdb.c:LINE` 注释
- 添加 UT：init 成功；rollover=true 时环形回绕；rollover=false 时空间耗尽返回 SAVED_FULL；max_len >= sec_size 断言失败

**Must NOT do**:
- 不要保留 `void*` 函数指针 lock/unlock
- 不要保留 control 命令模式 switch

**Recommended Agent Profile**:
- Category: `deep`

**Parallelization**:
- Can Run In Parallel: NO（依赖 T9, T15）
- Blocks: T17, T18, T22 (Final)
- Blocked By: T9 (Foundation), T15

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:866-1116` — TSDB init/control/clean

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib tsdb::tests::test_tsdb_init` — 初始化后 rollover=true, last_time=0
- [ ] `cargo test --lib tsdb::tests::test_tsdb_rollover` — 验证环形回绕行为
- [ ] `cargo test --lib tsdb::tests::test_tsdb_save_full` — 验证 non-rollover 满返回 SAVED_FULL

**QA Scenarios**:
```
Scenario: TSDB init + append + rollover
  Tool: Bash
  Steps:
    1. tsdb_init with 2 sectors, rollover=true
    2. append until sector 0 is full
    3. append triggers sector rollover to sector 1 (format sector 0)
    4. append continues, eventually wraps to sector 0 (format it)
  Expected Result: rollover 正确执行，old data 被覆盖
```

**Commit**: YES
- Message: `feat(tsdb): TSDB 核心函数完成（init, append, recovery, clean）+ UT`

---

### T17. TSDB 核心功能 3: iter, iter_reverse, iter_by_time, query_count, set_status, max_blob_count 翻译 + UT

**What to do**:
- 翻译 `fdb_tsdb.c:556-597` — `fdb_tsl_iter` 正序遍历
- 翻译 `fdb_tsdb.c:606-649` — `fdb_tsl_iter_reverse` 逆序遍历
- 翻译 `fdb_tsdb.c:654-768` — `search_start_tsl_addr`（二分查找起始 TSL）+ `fdb_tsl_iter_by_time` 范围查询
- 翻译 `fdb_tsdb.c:771-805` — `query_count_cb` + `fdb_tsl_query_count`
- 翻译 `fdb_tsdb.c:807-827` — `fdb_tsl_max_blob_count`
- 翻译 `fdb_tsdb.c:838-864` — `fdb_tsl_set_status` + `fdb_tsl_to_blob`
- 回调 `bool(cb)(tsl, arg)` → `impl FnMut(&Tsl) -> bool`
- 每个函数添加 `// c: fdb_tsdb.c:LINE` 注释
- 添加 UT：验证正序/逆序/范围查询的正确调用顺序；验证 query_count 统计；验证 set_status 后重读状态一致

**Must NOT do**:
- 不要简化二分查找逻辑
- 不要改变 from>to 表示反向遍历的语义

**Recommended Agent Profile**:
- Category: `deep`

**Parallelization**:
- Can Run In Parallel: NO（依赖 T15, T16）
- Blocks: T18, T22-T24 (Final)
- Blocked By: T15, T16

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:556-864` — TSDB 遍历与查询完整实现

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib tsdb::tests::test_tsl_iter` — 正序调用
- [ ] `cargo test --lib tsdb::tests::test_tsl_iter_reverse` — 逆序调用
- [ ] `cargo test --lib tsdb::tests::test_tsl_iter_by_time` — 范围查询
- [ ] `cargo test --lib tsdb::tests::test_query_count` — count 正确
- [ ] `cargo test --lib tsdb::tests::test_set_status` — 状态持久化

**QA Scenarios**:
```
Scenario: 正向遍历按时间升序
  Tool: Bash
  Steps:
    1. init + append 5 TSL: timestamps 100, 200, 300, 400, 500
    2. tsl_iter with callback collecting timestamps into Vec
    3. 验证 Vec == [100, 200, 300, 400, 500]
  Expected Result: 正序遍历时间戳升序

Scenario: 范围查询 [200, 400] 返回 3 条
  Tool: Bash
  Steps:
    1. iter_by_time(200, 400)
    2. 验证回调被调用 3 次，时间戳为 200, 300, 400
  Expected Result: 范围查询正确
```

**Commit**: NO（与 T18 末合并提交）

---

### T18. TSDB 核心功能 4: 综合验证（BDD 前置）+ 补充 UT

**What to do**:
- 补充之前未覆盖的边缘 case UT：
  - 多扇区 rollover
  - PRE_WRITE 恢复
  - 扇区切换时 end_info 保存
  - FIXED_BLOB_SIZE 模式
  - `fdb_tsl_max_blob_count` 边界
- 验证所有 TSDB 公共 API 完整覆盖（对照 `flashdb.h:62-71`）
- 运行 `cargo test --lib tsdb` 确保全部通过
- **C 测试等价性迁移**：将原 C 项目 `tests/` 中 TSDB 相关测试数据（append 顺序、iter 正逆序、范围查询边界、rollover 场景、query_count 统计）迁移为 `tests/c-port/tsdb_equiv.rs` integration test，对照 C 版输出验证行为等价性。这些测试数据是 C 作者验证过的金标准，用于捕获翻译错误

**Must NOT do**:
- 不要添加不存在的功能

**Recommended Agent Profile**:
- Category: `deep`

**Parallelization**:
- Can Run In Parallel: NO（依赖 T12, T16, T17）
- Blocks: T22, T23, T24 (Final)
- Blocked By: T12 (Final integration), T16, T17

**References**:
- `D:\MyCode\temp\FlashDB\inc\flashdb.h:62-71` — TSDB 公共 API 清单

**Acceptance Criteria**:
- [ ] `cargo test --lib` 全部通过
- [ ] 所有 TSDB API 至少 1 个 UT 覆盖

**QA Scenarios**:
```
Scenario: 完整 TSDB UT suite 通过
  Tool: Bash
  Steps:
    1. cargo test --lib tsdb → 全部通过，无 failures
    2. cargo test --lib tsdb -- --list → 列出所有测试，与 KVDB 测试数量相当
  Expected Result: 全部 UT 通过
```

**Commit**: YES
- Message: `feat(tsdb): TSDB 全功能实现完成（iter, range query, count, status, max_blob_count）+ 完整 UT`

---

## 5. UT 实现要求（§3.4 子 Plan 职责）

本 Plan 完成所有 TSDB 模块 UT，每个 UT 必须：
- 使用 `MockFlash`（来自 Foundation）作为 Flash 后端
- 每个 `assert!` 必须有真实业务逻辑验证（禁止 `assert!(true)`）
- 覆盖正常路径 + 错误路径 + 边界 case（空 TSL、扇区满、rollover、二分查找边界）

**函数级覆盖矩阵**：每个公共函数至少 3 个 case（normal / error / boundary）。完整覆盖矩阵见 Final Plan F4 审计引用的覆盖率矩阵，本 Plan 编码时按该矩阵补齐。

**UT 清单**：
| 测试函数 | 覆盖 Task | 验证内容 |
|---------|----------|---------|
| `tsdb::tests::test_header_layouts` | T14 | on-flash struct size + offset |
| `tsdb::tests::test_tsdb_sector_hdr_size` | T14 | TsdbSectorHdrData 大小 |
| `tsdb::tests::test_log_idx_size` | T14 | LogIdxData 大小 |
| `tsdb::tests::test_tsdb_offsets` | T14 | SECTOR_MAGIC_OFFSET, SECTOR_START_TIME_OFFSET 等 |
| `tsdb::tests::test_append_tsl` | T15 | append + read 往返一致 |
| `tsdb::tests::test_timestamp_ordering` | T15 | 时间戳单调递增 |
| `tsdb::tests::test_tsdb_init` | T16 | init 后状态正确 |
| `tsdb::tests::test_tsdb_rollover` | T16 | 环形回绕 |
| `tsdb::tests::test_tsdb_save_full` | T16 | non-rollover 满返回 SAVED_FULL |
| `tsdb::tests::test_tsl_iter` | T17 | 正序遍历 |
| `tsdb::tests::test_tsl_iter_reverse` | T17 | 逆序遍历 |
| `tsdb::tests::test_tsl_iter_by_time` | T17 | 范围查询 |
| `tsdb::tests::test_query_count` | T17 | count 统计 |
| `tsdb::tests::test_set_status` | T17 | 状态持久化 |
| `tsdb::tests::test_multi_sector_rollover` | T18 | 多扇区 rollover |
| `tsdb::tests::test_recovery_pre_write` | T18 | PRE_WRITE 恢复 |
| `tsdb::tests::test_end_info_save` | T18 | 扇区切换 end_info 保存 |

---

## 6. 出口条件（Plan 3 完成判定）

- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib tsdb` 全部 UT 通过
- [ ] `fdb_tsdb.c` 所有公共函数（`fdb_tsl_*`, `fdb_tsdb_*`）有对应 Rust pub fn（1:1 映射）
- [ ] 所有 on-flash struct（TsdbSectorHdrData, LogIdxData）size_of 编译期断言通过
- [ ] §2.2 下游契约中列出的公共 API 全部实现并可被 Final Plan 引用
- [ ] `cargo test --test c-port::tsdb_equiv` 通过（C 等价性验证）
- [ ] 两次 Commit 完成（T14 后 + T18 后）
