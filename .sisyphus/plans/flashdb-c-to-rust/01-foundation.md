# Plan 1: Foundation — 共享基础设施

> **总目标**：见 `00-start.md` — FlashDB C→Rust 1:1 翻译迁移
>
> **本 Plan 目标**：搭建 crate 骨架，翻译所有共享类型定义、底层工具 API、FlashDevice trait + MockFlash 实现、fdb.c 核心 init/deinit。本 Plan 产物是 Plan 2 (KVDB) 和 Plan 3 (TSDB) 的依赖根。

---

## 1. IN / OUT Scope

### IN Scope
- `Cargo.toml` — crate 配置 + features
- `src/lib.rs` — crate 根，模块声明 + pub use 重导出
- `src/def.rs` — 所有 enum/struct/类型定义（fdb_def.h + fdb_low_lvl.h 常量）
- `src/low_lvl.rs` — 常量宏、对齐宏、状态表操作、flash I/O wrapper、CRC32、blob API
- `src/flash_trait.rs` — `FlashDevice` trait 定义
- `src/mock_flash.rs` — `MockFlash` 内存模拟实现（测试用）
- `src/init.rs`（或 `src/lib.rs` 内）— fdb.c init/deinit/control 核心翻译

### OUT Scope
- ❌ KVDB 实现（Plan 2）
- ❌ TSDB 实现（Plan 3）
- ❌ BDD Step Definition（Final Plan）
- ❌ FAL 具体实现（第一阶段不实现）
- ❌ RT-Thread 集成层（丢弃）
- ❌ 文件模式 `fdb_file.c`（UT 用 MockFlash 替代）

---

## 2. 涉及目录/模块

```
src/
├── lib.rs           # crate 根, pub use 重导出
├── def.rs           # 类型定义 (fdb_def.h 翻译)
├── low_lvl.rs       # 底层 API (fdb_low_lvl.h + fdb.c + fdb_utils.c 翻译)
├── flash_trait.rs   # FlashDevice trait
└── mock_flash.rs    # MockFlash (#[cfg(test)])
Cargo.toml           # crate 配置 + features
```

---

## 3. 任务清单（T1-T9）

> 任务编号沿用原 Plan（T1-T9），保证溯源。本 Plan 内部依赖：T1 → T2,T3 → T4,T5 → T6,T7,T8,T9。

### T1. Cargo.toml scaffolding + crate 配置

**What to do**:
- 创建 `Cargo.toml`，crate 名 `flashdb`，edition 2021，`#![no_std]`
- 定义 Cargo features：`kvdb`（default）、`tsdb`（default）、`file_mode`、`kv_cache`、`kv_auto_update`、`timestamp_64bit`、`debug_enable`
- 配置 `FDB_WRITE_GRAN` 为 1（默认，可通过 feature `gran_8`/`gran_32`/`gran_64`/`gran_128`/`gran_256` 切换）
- dev-dependencies：`cucumber`（BDD）、`tokio`（cucumber runtime）
- dependencies：`serde`（可选，feature-gated）
- 创建 `src/lib.rs` — 声明所有模块，pub use 重导出公共 API

**Must NOT do**:
- 不要添加 RT-Thread 相关依赖
- 不要添加 FAL 具体实现依赖

**Recommended Agent Profile**:
- Category: `quick`

**Parallelization**:
- Can Run In Parallel: YES (与 T2-T5 并发，但 T2-T5 依赖 T1 完成)
- Blocks: 所有后续 tasks
- Blocked By: None

**References**:
- `D:\MyCode\temp\FlashDB\inc\fdb_cfg_template.h` — 所有配置项清单
- `D:\MyCode\temp\FlashDB\inc\flashdb.h` — 公共 API 列表

**Acceptance Criteria**:
- [ ] `cargo build` 成功（仅 crate 骨架）
- [ ] `cargo build --no-default-features` 成功
- [ ] `cargo build --features "kvdb,gran_8"` 成功

**QA Scenarios**:
```
Scenario: cargo build 所有 feature 组合编译通过
  Tool: Bash
  Steps:
    1. cargo build --features "kvdb"
    2. cargo build --features "tsdb"
    3. cargo build --features "kvdb,tsdb"
    4. cargo build --features "kvdb,gran_64"
  Expected Result: 所有命令 exit code = 0
```

**Commit**: NO（与 Wave 1 末合并提交）

---

### T2. def.rs — 所有 enum / struct / 类型定义翻译

**What to do**:
- 翻译 `fdb_def.h` 中所有类型定义到 `src/def.rs`
- 翻译 `fdb_low_lvl.h` 中的常量（`FDB_BYTE_ERASED=0xFF`, `FDB_BYTE_WRITTEN=0x00`, `FDB_ALIGN` 宏, `FDB_WG_ALIGN` 宏等）
- 每个 enum 使用 `#[repr(C)]`（跨 FFI）或 `#[repr(u8)]`（纯内部）
- 所有 struct（`fdb_kv`, `fdb_kv_iterator`, `fdb_tsl`, `kvdb_sec_info`, `tsdb_sec_info`, `kv_cache_node`, `fdb_db`, `fdb_kvdb`, `fdb_tsdb`, `fdb_blob`, `fdb_default_kv_node`, `fdb_default_kv`）必须 `#[repr(C)]`
- C 继承 `struct fdb_kvdb { struct fdb_db parent }` → 组合 + `impl AsRef<FdbDb> for FdbKvdb`
- `fdb_db` union storage → `enum Storage` trait bound
- `fdb_time_t` 根据 feature `timestamp_64bit` 使用 i32 或 i64
- **所有 on-flash struct 在文件末尾添加 `const _: () = assert!(core::mem::size_of::<T>() == EXPECTED)` 验证**
- 每个类型添加 `// c: fdb_def.h:LINE` 注释

**Must NOT do**:
- 不要将 `fdb_kvdb` 和 `fdb_tsdb` 合并到同一 .rs 文件
- 不要用 `Deref` 模拟继承
- 不要添加 `PhantomData` 到 on-flash struct
- 不要改变 on-flash 字节布局

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES (与 T1, T3-T5 同 Wave)
- Blocks: T6-T24
- Blocked By: T1

**References**:
- `D:\MyCode\temp\FlashDB\inc\fdb_def.h` — 完整类型定义（351 行）
- `D:\MyCode\temp\FlashDB\inc\fdb_low_lvl.h` — 常量和宏定义

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `#[repr(C)]` struct 的 size_of 验证在编译期通过
- [ ] 每个 enum / struct 有 `// c: fdb_def.h:LINE` 注释

**QA Scenarios**:
```
Scenario: size_of 验证全部通过
  Tool: Bash
  Steps:
    1. cargo build — 编译期 assert 触发表示布局错误
    2. 检查 src/def.rs 末尾有 const _: () 块验证每个 on-flash struct
  Expected Result: 编译成功，无 layout assertion 错误

Scenario: repr(C) 注解完整性检查
  Tool: Bash
  Steps:
    1. grep -n "repr" src/def.rs 列出所有 repr 注解
    2. 对比 C 源码中所有 struct/enum，确认每个都有 repr(C)
  Expected Result: 所有 on-flash 相关类型有 #[repr(C)]
```

**Commit**: NO（与 Wave 1 末合并提交）

---

### T3. FlashDevice trait 定义 + MockFlash 内存模拟实现

**What to do**:
- 创建 `src/flash_trait.rs`，定义 `trait FlashDevice`：
  ```rust
  pub trait FlashDevice {
      type Error;
      fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), Self::Error>;
      fn write(&mut self, addr: u32, buf: &[u8]) -> Result<(), Self::Error>;
      fn erase(&mut self, addr: u32, size: u32) -> Result<(), Self::Error>;
  }
  ```
- 创建 `src/mock_flash.rs`，实现 `MockFlash`：
  - 内部用 `Vec<u8>` 模拟 flash（初始全 0xFF）
  - 支持配置：sec_size, max_size, block_size, name
  - `write` 行为：只能 0xFF→0x00（模拟 NOR flash），不能直接覆盖（除非先 erase）
  - `erase` 行为：将 addr 起的 size 字节全部置为 0xFF
  - `read` 行为：直接返回 Vec 中的内容
- MockFlash 仅用于 `#[cfg(test)]`，但 trait 用于整个库
- 每个方法添加 `// c: port/fal/... or flashdb.h:LINE` 注释

**Must NOT do**:
- 不要在 trait 中引入 async/.await
- 不要用 `std::sync::Mutex`（考虑 no_std，用 `core::cell::RefCell` 或内部可变性）
- 不要在 MockFlash 中使用文件 I/O

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES (与 T1, T2, T4, T5 同 Wave)
- Blocks: T5, T9-T10, T14+
- Blocked By: T1

**References**:
- `D:\MyCode\temp\FlashDB\port\fal\` — FAL vtable 参考
- `D:\MyCode\temp\FlashDB\inc\fdb_def.h:264-294` — fdb_db union storage 参考

**Acceptance Criteria**:
- [ ] `trait FlashDevice` 有 read/write/erase 三个方法
- [ ] `MockFlash` 实现所有三个方法
- [ ] MockFlash 初始全 0xFF
- [ ] MockFlash write 不能覆盖已写内容（模拟 NOR flash 特性）
- [ ] MockFlash erase 将区域重置为 0xFF

**QA Scenarios**:
```
Scenario: MockFlash read/write/erase 基本行为正确
  Tool: Bash
  Preconditions: MockFlash sec_size=4096, max_size=16384
  Steps:
    1. 创建 MockFlash，验证初始全 0xFF（read 返回全 FF）
    2. write(addr=0, buf=[0x00, 0x01, 0x02])
    3. read(addr=0, 3 bytes) → 返回 [0x00, 0x01, 0x02]
    4. write(addr=1, [0xAA]) → 返回 Err（覆盖已写内容）
    5. erase(addr=0, 4096) → 返回 Ok
    6. read(addr=0, 3 bytes) → 返回 [0xFF, 0xFF, 0xFF]
  Expected Result: 所有操作符合 NOR flash 语义
```

**Commit**: NO（与 Wave 1 末合并提交）

---

### T4. low_lvl.rs — 常量宏 + 对齐宏 + 状态表操作翻译

**What to do**:
- 翻译 `fdb_low_lvl.h` 中所有宏为 const / inline fn：
  - `FDB_BYTE_ERASED=0xFF` → `pub(crate) const BYTE_ERASED: u8 = 0xFF;`
  - `FDB_BYTE_WRITTEN=0x00` → `pub(crate) const BYTE_WRITTEN: u8 = 0x00;`
  - `FDB_ALIGN(size, align)` → `pub(crate) const fn align_up(size: u32, align: u32) -> u32`
  - `FDB_WG_ALIGN(size)` → `pub(crate) const fn wg_align(size: u32) -> u32` (依赖 GRAN const generic)
  - `FDB_WG_ALIGN_DOWN(size)` → `pub(crate) const fn wg_align_down(size: u32) -> u32`
  - `FDB_STATUS_TABLE_SIZE(n)` → `pub(crate) const fn status_table_size(n: u32) -> u32`
  - `FDB_DATA_UNUSED=0xFFFFFFFF` → `pub(crate) const DATA_UNUSED: u32 = 0xFFFFFFFF;`
  - `FDB_FAILED_ADDR=0xFFFFFFFF` → `pub(crate) const FAILED_ADDR: u32 = 0xFFFFFFFF;`
- 翻译 `fdb_utils.c` 中 `_fdb_set_status` 和 `_fdb_get_status`：
  - 按 SKILL §1-A 规则：switch→match, memset→buf.fill, for(i=0;i<n;i++)→for i in 0..n
  - 保持与 C 版完全一致的 bit 编码逻辑
- 每个函数添加 `// c: fdb_utils.c:LINE` 注释
- 添加 UT 验证状态表编码：对 FDB_WRITE_GRAN=1 和 GRAN>1 分别测试

**Must NOT do**:
- 不要将状态表逻辑改为运行时 if（必须用 const generic）
- 不要简化 status encoding 表

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES (与 T1, T2, T3, T5 同 Wave)
- Blocks: T5-T9, T10+
- Blocked By: T1, T2

**References**:
- `D:\MyCode\temp\FlashDB\inc\fdb_low_lvl.h:18-55` — 宏定义完整列表
- `D:\MyCode\temp\FlashDB\src\fdb_utils.c:91-145` — set_status/get_status 实现

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib low_lvl` 状态表 UT 全部通过
- [ ] WRAN=1 时 status table 编码与 C 版一致
- [ ] GRAN=8/32/64 时同样正确

**QA Scenarios**:
```
Scenario: set_status GRAN=1 编码正确
  Tool: Bash
  Steps:
    1. cargo test --lib low_lvl::tests::test_set_status_gran1 -- 调用 set_status(table, 4, 0/1/2/3)
    2. 对照 C 版：index=0 → 全 FF；index=1 → 0x7F；index=2 → 0x3F；index=3 → 0x1F
  Expected Result: 每个编码值与 C 版完全一致

Scenario: get_status GRAN=1 解码正确
  Tool: Bash
  Steps:
    1. cargo test --lib low_lvl::tests::test_get_status_gran1
  Expected Result: 解码后的 index 与 C 版一致

Scenario: align_up / wg_align / wg_align_down 计算正确
  Tool: Bash
  Steps:
    1. cargo test --lib low_lvl::tests::test_align_macros
    2. 验证 FDB_ALIGN(13,4)=16, FDB_WG_ALIGN(13)=16 (GRAN=8), FDB_WG_ALIGN_DOWN(15)=16
  Expected Result: 所有对齐计算与 C 版一致
```

**Commit**: NO（与 Wave 1 末合并提交）

---

### T5. low_lvl.rs — flash I/O wrapper (read/erase/write/aligned write)

**What to do**:
- 翻译 `fdb_utils.c` 中的 flash 操作函数：
  - `_fdb_flash_read(db, addr, buf, size)` → `pub(crate) fn flash_read(flash: &mut F, addr: u32, buf: &mut [u8]) -> Result<(), FdbErr>`
  - `_fdb_flash_erase(db, addr, size)` → `pub(crate) fn flash_erase(flash: &mut F, addr: u32, size: u32) -> Result<(), FdbErr>`
  - `_fdb_flash_write(db, addr, buf, size, sync)` → `pub(crate) fn flash_write(flash: &mut F, addr: u32, buf: &[u8], sync: bool) -> Result<(), FdbErr>`
  - `_fdb_flash_write_align(db, addr, buf, size)` → `pub(crate) fn flash_write_align(flash: &mut F, addr: u32, buf: &[u8]) -> Result<(), FdbErr>`
- 翻译 `fdb_utils.c:185-210` `_fdb_continue_ff_addr` — 查找连续 0xFF 地址
- 所有函数签名添加 `// c: fdb_utils.c:LINE` 注释
- `_fdb_flash_write_align` 中对齐补全逻辑保持与 C 一致
- 添加 UT 使用 MockFlash 验证 read/erase/write/aligned write 行为

**Must NOT do**:
- 不要在 flash_write 中添加 sync 参数（Rust 用 flush/explicit API）
- 不要用 unsafe 读写 flash

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES (与 T1-T4 同 Wave)
- Blocks: T6-T9, T10+
- Blocked By: T1, T2, T3, T4

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_utils.c:257-349` — flash 操作函数完整实现
- `D:\MyCode\temp\FlashDB\src\fdb_utils.c:185-210` — continue_ff_addr 实现
- `src/flash_trait.rs` — FlashDevice trait 定义

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib low_lvl::tests::test_flash_io` 通过
- [ ] flash_write_align 对齐补全逻辑与 C 一致
- [ ] continue_ff_addr 在已知数据上返回正确地址

**QA Scenarios**:
```
Scenario: MockFlash read/write/erase/aligned write 完整流程
  Tool: Bash
  Preconditions: MockFlash with GRAN=8, sec_size=4096
  Steps:
    1. erase(0, 4096) → 检查全 0xFF
    2. write(0, [0x00, 0x01, 0x02, 0x03]) → 返回 Ok
    3. read(0, 4) → 返回 [0x00, 0x01, 0x02, 0x03]
    4. write_align(4, [0xAA, 0xBB, 0xCC]) → 返回 Ok（GRAN=8, 4 bytes aligned to 8）
    5. read(4, 8) → 前 3 字节是 [0xAA, 0xBB, 0xCC]，第 4 字节是 0xFF padding
  Expected Result: aligned write 正确补 0xFF padding

Scenario: continue_ff_addr 与 C 版一致
  Tool: Bash
  Steps:
    1. 在 MockFlash 写入数据：[0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF]
    2. 调用 continue_ff_addr(flash, 0, 9)
    3. C 版结果应为 wg_align(7) = 8 (GRAN=8)
  Expected Result: 返回地址与 C 版完全一致
```

**Commit**: YES（Wave 1 末合并提交）
- Message: `feat(scaffold): crate 骨架 + 类型定义 + FlashDevice trait + 底层 API + MockFlash`

---

### T6. CRC32 模块翻译 + UT

**What to do**:
- 翻译 `fdb_utils.c:21-89` 中的 CRC32 实现到 `src/def.rs` 或独立 `src/crc32.rs`
- 保持 256 项 CRC32 查找表完全不变
- `fdb_calc_crc32` → `pub fn calc_crc32(crc: u32, buf: &[u8]) -> u32`
- 使用 C 原版的 crc32_table 数据（逐字节异或逻辑）
- 添加 UT 验证：空输入、已知字符串、二进制数据 vs C 版输出
- 函数签名添加 `// c: fdb_utils.c:77-89` 注释

**Must NOT do**:
- 不要使用第三方 crc crate（必须 1:1 翻译）
- 不要改变 CRC 多项式或查找表

**Recommended Agent Profile**:
- Category: `quick`

**Parallelization**:
- Can Run In Parallel: YES (与 T7-T9 同 Wave)
- Blocks: T10, T11, T14+
- Blocked By: T1, T2, T4

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_utils.c:21-89` — CRC32 查表 + 计算函数

**Acceptance Criteria**:
- [ ] `cargo test --lib crc32` 全部通过
- [ ] 空串 CRC = 0x00000000
- [ ] "Hello" CRC = 0xABF77660（已知值）
- [ ] 二进制数据（全 0xFF）CRC 与 C 版一致

**QA Scenarios**:
```
Scenario: CRC32 与 C 版输出完全一致
  Tool: Bash
  Steps:
    1. cargo test --lib crc32::tests::test_empty → 0x00000000
    2. cargo test --lib crc32::tests::test_hello → 已知值
    3. cargo test --lib crc32::tests::test_binary_ff → 对比 C 版
  Expected Result: 所有 CRC 值与 C 版一致
```

**Commit**: NO（与 Wave 2 末合并提交）

---

### T7. write_status / read_status + 状态持久化 UT

**What to do**:
- 翻译 `fdb_utils.c:147-180` 中的 `_fdb_write_status` 和 `_fdb_read_status`
- `_fdb_write_status` → `pub(crate) fn write_status(flash, addr, status_num, status_index, sync) -> Result<(), FdbErr>`
- `_fdb_read_status` → `pub(crate) fn read_status(flash, addr, total_num) -> u32`
- 依赖 T4 的 set_status/get_status 和 T5 的 flash I/O
- 添加 UT：用 MockFlash 验证写入状态表后 read_back 解码正确
- 添加 UT：验证状态表在不同 WRITE_GRAN 下的行为

**Must NOT do**:
- 不要将状态表改为 HashMap/BTreeMap

**Recommended Agent Profile**:
- Category: `quick`

**Parallelization**:
- Can Run In Parallel: YES (与 T6, T8, T9 同 Wave)
- Blocks: T10-T16
- Blocked By: T4, T5

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_utils.c:147-180` — write_status/read_status 完整实现

**Acceptance Criteria**:
- [ ] `cargo test --lib low_lvl::tests::test_write_read_status` 通过
- [ ] write+read 往返解码结果一致

**QA Scenarios**:
```
Scenario: write_status 后 read_status 解码一致
  Tool: Bash
  Steps:
    1. write_status(flash, addr=0, total=4, index=2) (GRAN=1)
    2. read_status(flash, addr=0, total=4)
    3. 期望解码 index=2
  Expected Result: read 返回的 index 与写入时相同
```

**Commit**: NO（与 Wave 2 末合并提交）

---

### T8. blob API 翻译 + UT

**What to do**:
- 翻译 `fdb_def.h:335-344` 中 `struct fdb_blob`（已在 T2 完成）
- 翻译 `fdb_utils.c:221-249` 中的 blob API：
  - `fdb_blob_make(blob, value_buf, buf_len)` → `pub fn blob_make(buf: &mut [u8]) -> Blob`
  - `fdb_blob_read(db, blob)` → `pub fn blob_read(flash: &mut F, blob: &mut Blob) -> usize`
- Blob 是通用数据传输容器，在 KVDB/TSDB 中大量使用
- 每个函数添加 `// c: fdb_utils.c:LINE` 注释
- 添加 UT：验证 blob_make/blob_read 行为

**Must NOT do**:
- 不要将 Blob 改为 `Cow<T>`（保持简单）

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES (与 T6, T7, T9 同 Wave)
- Blocks: T11, T15+
- Blocked By: T4, T5

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb_utils.c:212-249` — blob_make/blob_read 实现

**Acceptance Criteria**:
- [ ] `cargo test --lib blob` 全部通过
- [ ] blob_read 超出 saved.len 时正确截断

**QA Scenarios**:
```
Scenario: blob_make + blob_read 往返一致
  Tool: Bash
  Steps:
    1. blob_make: buf = 32 bytes of [0x01..0x20]
    2. MockFlash write 到 saved.addr
    3. blob_read(flash, &mut blob)
    4. 验证 buf 内容与写入一致
  Expected Result: 读取数据与写入数据完全一致
```

**Commit**: NO（与 Wave 2 末合并提交）

---

### T9. fdb.c — init/deinit/control 核心翻译 + UT

**What to do**:
- 翻译 `fdb.c:31-157` 中的核心初始化逻辑：
  - `_fdb_init_ex(db, name, path, type, user_data)` → `pub(crate) fn init_ex(db: &mut Db, name, path, flash, ...) -> Result<(), FdbErr>`
  - `_fdb_init_finish(db, result)` → `pub(crate) fn init_finish(db: &mut Db, result)`
  - `_fdb_deinit(db)` → `pub(crate) fn deinit(db: &mut Db)`
  - `_fdb_db_path(db)` → `pub(crate) fn db_path(db: &Db) -> &str`
- FAL 部分（`if (!file_mode)`) 翻译为 trait bound 版本
- `log_is_show` static → `AtomicBool`
- `FDB_ASSERT(db->sec_size != 0)` → `assert!` 宏
- 每个函数添加 `// c: fdb.c:LINE` 注释
- 添加 UT：验证 init_ex 的各种错误路径（分区不存在、大小不对齐、扇区不足）
- **C 测试等价性迁移**：将原 C 项目 `tests/` 中 Foundation 层相关测试数据（CRC32 已知输出、status table 编码表、blob 往返）迁移为 `tests/c-port/foundation_equiv.rs` integration test，对照 C 版输出验证行为等价性。这些测试数据是 C 作者验证过的金标准，用于捕获翻译错误

**Must NOT do**:
- 不要保留 FAL 具体调用（fal_init/fal_partition_find 等）
- 不要简化 sec_size 验证逻辑

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES (与 T6-T8 同 Wave)
- Blocks: T10, T13, T16
- Blocked By: T1, T2, T3, T4, T5

**References**:
- `D:\MyCode\temp\FlashDB\src\fdb.c:31-157` — 完整 init/deinit 实现

**Acceptance Criteria**:
- [ ] `cargo build` 编译通过
- [ ] `cargo test --lib init::tests` 全部通过
- [ ] init_ex 校验 sec_size 是 2 的幂

**QA Scenarios**:
```
Scenario: init_ex 校验参数错误
  Tool: Bash
  Steps:
    1. init_ex with sec_size=3000（非 2 的幂） → panic（assert 触发）
    2. init_ex with max_size=5000, sec_size=4096（非整数倍）→ InitFailed
    3. init_ex with max_size=4096, sec_size=4096（仅 1 扇区）→ InitFailed
  Expected Result: 所有错误路径正确返回对应错误码

Scenario: init_ex 正常初始化
  Tool: Bash
  Steps:
    1. init_ex with name="test", sec_size=4096, max_size=16384 (4 sectors)
    2. 验证 db.init_ok = true
  Expected Result: 初始化成功，init_ok 为 true
```

**Commit**: YES（Wave 2 末合并提交）
- Message: `feat(core): CRC32 模块完成, 状态表操作完成, blob API 完成, init/deinit 核心完成`

---

## 4. UT 实现要求（§3.4 子 Plan 职责）

本 Plan 完成所有 Foundation 层 UT，每个 UT 必须：
- 使用 `MockFlash` 作为 Flash 后端
- 每个 `assert!` 必须有真实业务逻辑验证（禁止 `assert!(true)`）
- 覆盖正常路径 + 至少 1 个错误路径
- GRAN=1 和 GRAN=8 两种配置下都验证（对齐/状态表相关）

**函数级覆盖矩阵**：每个公共函数至少 3 个 case（normal / error / boundary）。完整覆盖矩阵见 Final Plan F4 审计引用的覆盖率矩阵，本 Plan 编码时按该矩阵补齐。

**UT 清单**：
| 测试函数 | 覆盖 Task | 验证内容 |
|---------|----------|---------|
| `mock_flash::tests::*` | T3 | NOR flash 语义 |
| `low_lvl::tests::test_set_status_gran1` | T4 | GRAN=1 状态表编码 |
| `low_lvl::tests::test_get_status_gran1` | T4 | GRAN=1 状态表解码 |
| `low_lvl::tests::test_align_macros` | T4 | 对齐宏计算 |
| `low_lvl::tests::test_flash_io` | T5 | flash I/O 完整流程 |
| `low_lvl::tests::test_continue_ff` | T5 | 连续 FF 查找 |
| `crc32::tests::test_empty/hello/binary_ff` | T6 | CRC32 字节级一致 |
| `low_lvl::tests::test_write_read_status` | T7 | 状态表往返 |
| `blob::tests::*` | T8 | blob make/read |
| `init::tests::test_init_errors` | T9 | init 错误路径 |
| `init::tests::test_init_success` | T9 | init 正常路径 |

---

## 5. 出口条件（Plan 1 完成判定）

- [ ] `cargo build` 编译通过，零 warning
- [ ] `cargo test --lib` 全部 UT 通过
- [ ] 所有 on-flash struct size_of 编译期断言通过
- [ ] Foundation 公共 API（见 `00-start.md` §4.2）全部可被 Plan 2/3 引用
- [ ] `cargo test --test c-port::foundation_equiv` 通过（C 等价性验证）
- [ ] 两次 Commit 完成（Wave 1 末 + Wave 2 末）
