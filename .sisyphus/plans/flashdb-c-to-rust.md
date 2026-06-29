# FlashDB C→Rust 1:1 翻译迁移计划

## TL;DR

> **Quick Summary**: 将 FlashDB C 源码（KVDB + TSDB，~3600 行 C）1:1 翻译为 Rust 嵌入式库，保持 on-flash 格式字节级兼容，并配套单元测试和 BDD 验收测试。
>
> **Deliverables**:
> - 完整 Rust `flashdb` crate（`no_std` 兼容）
> - FlashDevice trait + 内存模拟实现（测试用）
> - KVDB 全功能：init, CRUD, iterator, GC, recovery
> - TSDB 全功能：init, append, iter/iter_reverse/iter_by_time, query, clean, status
> - CRC32 工具模块
> - 单元测试（禁止虚假断言）
> - BDD Gherkin 测试用例（基于现有 6 个 .feature 文件）
>
> **Estimated Effort**: Large
> **Parallel Execution**: YES — 5 waves（最大并发 7）
> **Critical Path**: Task 1 → Task 5 → Task 10 → Task 15 → Task 20 → Final QA

---

## Context

### Original Request
将开源 FlashDB（`D:\MyCode\temp\FlashDB`）从 C 到 Rust 进行 1:1 代码翻译，确保规格不遗漏，遵从 Rust 语言特点。基于已有的 harness/features 生成 BDD 测试用例，UT 测试禁止虚假断言。

### Interview Summary
**Key Discussions**:
- 默认 1:1 翻译，所有 on-flash struct 必须 `#[repr(C)]` + `size_of` 验证
- §2 建议项全部通过：FAL→trait, GRAN→const generic, control→Builder
- 使用 c-translate-to-rust skill 作为翻译规则基线
- RT-Thread 集成层丢弃，用 embedded-storage 替代
- 第一阶段 FAL 不实现，仅实现内存模拟 + 文件模式（可选）

**Research Findings**:
- C 源码共 5 文件：fdb.c(157L), fdb_utils.c(349L), fdb_kvdb.c(1944L), fdb_tsdb.c(1118L), fdb_file.c(317L)
- 公共 API 共 30+ 函数（KVDB 15, TSDB 13, 通用 4）
- 已有 6 个 Gherkin feature 文件 + 2 个 spec 文件 + C 测试文件
- Skill 14 个 reference 文档覆盖所有复杂翻译场景

### Metis Review
**Identified Gaps** (addressed):
- 所有 `#[repr(C)]` struct 必须加 `size_of` 校验 → 纳入每个 on-flash struct task
- 文件模式（fdb_file.c）在 UT 中用内存模拟替代 → Mock Flash 实现
- 函数名溯源 → 每个 fn 注释 `// c: xxx.c:line`
- FAL 适配层单独 task → 不强制第一阶段
- 测试数据对齐 → Mock Flash 返回对齐数据
- BDD 场景与 feature 文件映射 → BDD task 中明确

---

## Work Objectives

### Core Objective
完成 FlashDB C 代码到 Rust 的 1:1 翻译，保持 on-flash 二进制格式兼容，并通过测试验证。

### Concrete Deliverables
- `src/lib.rs` — crate 根，pub use 重导出
- `src/def.rs` — 所有类型定义（fdb_def.h 翻译）
- `src/low_lvl.rs` — 底层 API、宏、状态表操作（fdb_low_lvl.h + fdb.c + fdb_utils.c）
- `src/kvdb.rs` — KVDB 完整实现（fdb_kvdb.c 翻译）
- `src/tsdb.rs` — TSDB 完整实现（fdb_tsdb.c 翻译）
- `src/flash_trait.rs` — FlashDevice trait
- `src/mock_flash.rs` — 内存模拟 Flash（测试用）
- `Cargo.toml` — features 配置（WRITE_GRAN, FILE_MODE 等）
- `tests/` — BDD cucumber 测试

### Definition of Done
- [ ] `cargo build` 成功
- [ ] `cargo test` 所有 UT 通过
- [ ] 所有 on-flash struct size_of 验证通过
- [ ] BDD 场景全部覆盖（与 feature 文件 1:1 对应）
- [ ] 零 `unsafe` 在非 FFI 边界使用

### Must Have
- 1:1 函数映射，每个 fn 标注 C 源码位置
- 所有 on-flash struct `#[repr(C)]` + `size_of` 编译期验证
- CRC32 与 C 版字节级一致
- 所有条件编译 → Cargo features（禁止运行时 if）
- UT 每个断言有真实业务逻辑验证（禁止虚假断言）

### Must NOT Have (Guardrails)
- ❌ on-flash 二进制格式变更
- ❌ `unsafe { transmute }` 在非 FFI 边界
- ❌ `unwrap()` / `expect()` 对 Result
- ❌ `impl Deref for Child` 模拟继承
- ❌ `unimplemented!()` / `todo!()` 占位
- ❌ `_` 前缀变量名
- ❌ 合并多个 struct 到同一文件
- ❌ 简化/硬编码/伪造默认值

---

## Verification Strategy (MANDATORY)

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

### Test Decision
- **Infrastructure exists**: YES（cargo 原生）
- **Automated tests**: Tests-after（先翻译后补测试）
- **Framework**: `cargo test` + `cucumber-rust`（BDD）
- **Mock**: 内存模拟 Flash（`MockFlash` 实现 `FlashDevice` trait）

### QA Policy
每个 task 包含 agent-executed QA 场景：
- 单元测试：`cargo test` 命令验证
- 类型验证：`cargo build` 编译通过
- 布局验证：编译期 `size_of` 常量断言

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately — foundation + scaffolding):
├── Task 1:  Cargo.toml scaffolding + features
├── Task 2:  def.rs — 所有 enum / struct / 类型定义
├── Task 3:  FlashDevice trait 定义 + Mock Flash
├── Task 4:  low_lvl.rs — 常量宏 + size_of 验证
└── Task 5:  low_lvl.rs — flash I/O wrapper (read/erase/write)

Wave 2 (After Wave 1 — core modules, MAX PARALLEL):
├── Task 6:  CRC32 模块 + UT
├── Task 7:  状态表操作 (set_status / get_status) + UT
├── Task 8:  fdb blob API (make/read) + UT
├── Task 9:  fdb.c init/deinit/control 核心 + UT
└── Task 10: kvdb.rs — KV header + sector header on-flash struct + UT

Wave 3 (After Wave 2 — KVDB & TSDB impl):
├── Task 11: KVDB impl: read_kv, find_kv, get_kv + UT
├── Task 12: KVDB impl: create_kv, set_kv, del_kv + UT
├── Task 13: KVDB impl: iterator, gc, set_default + UT
├── Task 14: TSDB on-flash struct (sector_hdr, log_idx) + UT
├── Task 15: TSDB impl: read_tsl, format_sector, write_tsl + UT
└── Task 16: TSDB impl: init + control + UT

Wave 4 (After Wave 3 — TSDB query + BDD):
├── Task 17: TSDB impl: iter, iter_reverse, iter_by_time + UT
├── Task 18: TSDB impl: query_count, set_status, clean + UT
├── Task 19: BDD cucumber-rust 集成框架搭建
├── Task 20: BDD 场景实现: KVDB init + CRUD (2 feature files)
└── Task 21: BDD 场景实现: KVDB iteration + GC (1 feature file)

Wave 5 (After Wave 4 — TSDB BDD + final):
├── Task 22: BDD 场景实现: TSDB init (1 feature file)
├── Task 23: BDD 场景实现: TSDB append (1 feature file)
└── Task 24: BDD 场景实现: TSDB query + management (1 feature file)

Wave FINAL (After ALL tasks):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real manual QA (unspecified-high)
└── Task F4: Scope fidelity check (deep)
```

### Dependency Matrix

- **T1**: — → T2,T3,T4,T5,T6+
- **T2**: T1 → T3,T4,T5,T10,T14
- **T3**: T1,T2 → T9,T10,T14+
- **T4**: T1,T2,T3 → T5,T6,T7,T8
- **T5**: T1,T2,T3 → T6,T7,T8,T9
- **T6**: T4,T5 → T10,T11,T14
- **T7**: T4,T5 → T10,T14+
- **T8**: T4,T5 → T11,T15
- **T9**: T5,T8 → T10,T13,T16
- **T10**: T2,T3,T4,T5,T6 → T11,T12
- **T11**: T7,T8,T10 → T12
- **T12**: T11,T6 → T13,T18
- **T13**: T6,T9,T12 → T20,T21
- **T14**: T2,T3,T6,T7 → T15,T16
- **T15**: T7,T8,T14 → T16,T17
- **T16**: T9,T15 → T17,T18,T22
- **T17**: T16 → T18,T22
- **T18**: T12,T16,T17 → T22,T23,T24
- **T19**: T1,T20,T21,T22+ → F1-F4
- **T20**: T13,T19 → F1-F4
- **T21**: T13,T19 → F1-F4
- **T22**: T18,T19 → F1-F4
- **T23**: T18,T19 → F1-F4
- **T24**: T18,T19 → F1-F4

### Agent Dispatch Summary

- **Wave 1**: 5 tasks — T1-T5 → `quick`
- **Wave 2**: 5 tasks — T6 → `quick`, T7 → `quick`, T8 → `unspecified-high`, T9 → `unspecified-high`, T10 → `quick`
- **Wave 3**: 6 tasks — T11 → `deep`, T12 → `deep`, T13 → `deep`, T14 → `quick`, T15 → `deep`, T16 → `deep`
- **Wave 4**: 5 tasks — T17 → `deep`, T18 → `deep`, T19 → `quick`, T20 → `unspecified-high`, T21 → `unspecified-high`
- **Wave 5**: 3 tasks — T22 → `unspecified-high`, T23 → `unspecified-high`, T24 → `unspecified-high`
- **FINAL**: 4 tasks — F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs


- [ ] 1. Cargo.toml scaffolding + crate 配置

  **What to do**:
  - 创建 `Cargo.toml`，crate 名 `flashdb`，edition 2021，`#![no_std]`
  - 定义 Cargo features：`kvdb`（default）、`tsdb`（default）、`file_mode`、`kv_cache`、`kv_auto_update`、`timestamp_64bit`、`debug_enable`
  - 配置 `FDB_WRITE_GRAN` 为 1（默认，可通过 feature `gran_8`/`gran_32`/`gran_64`/`gran_128`/`gran_256` 切换）
  - 添加 dev-dependencies：`cucumber`（BDD）、`tokio`（cucumber runtime）
  - 添加 dependencies：`serde`（可选，feature-gated）
  - 创建 `src/lib.rs` — crate 根文件，声明所有模块，pub use 重导出公共 API

  **Must NOT do**:
  - 不要添加 RT-Thread 相关依赖
  - 不要添加 FAL 具体实现依赖

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: Cargo features 映射 `#ifdef`，const generic 配置 WRITE_GRAN

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2-5)
  - **Blocks**: 所有后续 tasks
  - **Blocked By**: None

  **References**:
  - `D:\MyCode\temp\FlashDB\inc\fdb_cfg_template.h` — 所有配置项清单，映射到 Cargo features
  - `D:\MyCode\temp\FlashDB\inc\flashdb.h` — 公共 API 列表，用于 lib.rs pub use
  - `references/feature-flags.md`（c-translate-to-rust skill）— Cargo features 映射 WRITE_GRAN 方案

  **Acceptance Criteria**:
  - [ ] `cargo build` 成功（仅 crate 骨架）
  - [ ] `cargo build --no-default-features` 成功
  - [ ] `cargo build --features "kvdb,gran_8"` 成功

  **QA Scenarios**:
  ```
  Scenario: cargo build 所有 feature 组合编译通过
    Tool: Bash
    Steps:
      1. `cargo build --features "kvdb"`
      2. `cargo build --features "tsdb"`
      3. `cargo build --features "kvdb,tsdb"`
      4. `cargo build --features "kvdb,gran_64"`
    Expected Result: 所有命令 exit code = 0
    Evidence: .sisyphus/evidence/task-1-cargo-build.log
  ```

  **Evidence to Capture**:
  - [ ] task-1-cargo-build.log

  **Commit**: NO (groups with Wave 1)

- [ ] 2. def.rs — 所有 enum / struct / 类型定义翻译

  **What to do**:
  - 翻译 `fdb_def.h` 中所有类型定义到 `src/def.rs`
  - 翻译 `fdb_low_lvl.h` 中的常量（FDB_BYTE_ERASED=0xFF, FDB_BYTE_WRITTEN=0x00, FDB_ALIGN 宏, FDB_WG_ALIGN 宏等）
  - 每个 enum 使用 `#[repr(C)]`（跨 FFI）或 `#[repr(u8)]`（纯内部）
  - 所有 struct（fdb_kv, fdb_kv_iterator, fdb_tsl, kvdb_sec_info, tsdb_sec_info, kv_cache_node, fdb_db, fdb_kvdb, fdb_tsdb, fdb_blob, fdb_default_kv_node, fdb_default_kv）必须 `#[repr(C)]`
  - C 继承 `struct fdb_kvdb { struct fdb_db parent }` → 组合 + `impl AsRef<FdbDb> for FdbKvdb`
  - fdb_db union storage → `enum Storage` trait bound
  - `fdb_time_t` 根据 feature `timestamp_64bit` 使用 i32 或 i64
  - **所有 on-flash struct 在文件末尾添加 `assert_eq!(core::mem::size_of::<T>(), EXPECTED)` 验证**
  - 每个类型添加 `// c: fdb_def.h:LINE` 注释

  **Must NOT do**:
  - 不要将 fdb_kvdb 和 fdb_tsdb 合并到同一 .rs 文件
  - 不要用 Deref 模拟继承
  - 不要添加 PhantomData 到 on-flash struct
  - 不要改变 on-flash 字节布局

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1 强制翻译表 + §1-B 复杂规则（c-inheritance.md, conditional-struct-layout.md, on-flash-compat.md）

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3-5)
  - **Blocks**: Tasks 6-24
  - **Blocked By**: Task 1 (Cargo.toml)

  **References**:
  - `D:\MyCode\temp\FlashDB\inc\fdb_def.h` — 完整类型定义（351 行）
  - `D:\MyCode\temp\FlashDB\inc\fdb_low_lvl.h` — 常量和宏定义
  - `references/c-inheritance.md` — C 继承到组合+AsRef 翻译方案
  - `references/conditional-struct-layout.md` — FDB_WRITE_GRAN padding 方案
  - `references/on-flash-compat.md` — on-flash 布局兼容性验证

  **Acceptance Criteria**:
  - [ ] `cargo build` 编译通过
  - [ ] `#[repr(C)]` struct 的 size_of 验证在编译期通过
  - [ ] 每个 enum / struct 有 `// c: fdb_def.h:LINE` 注释

  **QA Scenarios**:
  ```
  Scenario: size_of 验证全部通过
    Tool: Bash
    Steps:
      1. `cargo build` — 编译期 assert 触发表示布局错误
      2. 检查 src/def.rs 末尾有 `const _: ()` 块验证每个 on-flash struct
    Expected Result: 编译成功，无 layout assertion 错误
    Evidence: .sisyphus/evidence/task-2-size-verify.log

  Scenario: repr(C) 注解完整性检查
    Tool: Bash
    Steps:
      1. grep -n "repr" src/def.rs 列出所有 repr 注解
      2. 对比 C 源码中所有 struct/enum，确认每个都有 repr(C)
    Expected Result: 所有 on-flash 相关类型有 #[repr(C)]
    Evidence: .sisyphus/evidence/task-2-repr-check.txt
  ```

  **Evidence to Capture**:
  - [ ] task-2-size-verify.log
  - [ ] task-2-repr-check.txt

  **Commit**: NO (groups with Wave 1)

- [ ] 3. FlashDevice trait 定义 + MockFlash 内存模拟实现

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
  - **Category**: `unspecified-high`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B fal-to-trait.md (FlashDevice trait 设计参考)
    - `c-translate-to-rust`: no_std 适配规则

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-2, 4-5)
  - **Blocks**: Tasks 5, 9-10, 14+
  - **Blocked By**: Task 1

  **References**:
  - `D:\MyCode\temp\FlashDB\port\fal\` — FAL vtable 参考（函数指针表）
  - `D:\MyCode\temp\FlashDB\inc\fdb_def.h:264-294` — fdb_db union storage 参考
  - `references/fal-to-trait.md` — FAL 函数指针 → FlashDevice trait 翻译方案
  - `references/on-flash-compat.md` — MockFlash 需模拟的 flash 特性

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
    Evidence: .sisyphus/evidence/task-3-mock-flash-test.log
  ```

  **Evidence to Capture**:
  - [ ] task-3-mock-flash-test.log

  **Commit**: NO (groups with Wave 1)

- [ ] 4. low_lvl.rs — 常量宏 + 对齐宏 + 状态表操作翻译

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
  - **Category**: `unspecified-high`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-A 强制映射表（const, match, for loop）
    - `c-translate-to-rust`: §1-B feature-flags.md（WRITE_GRAN 编码变体）

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-3, 5)
  - **Blocks**: Tasks 5-9, 10+
  - **Blocked By**: Tasks 1, 2

  **References**:
  - `D:\MyCode\temp\FlashDB\inc\fdb_low_lvl.h:18-55` — 宏定义完整列表
  - `D:\MyCode\temp\FlashDB\src\fdb_utils.c:91-145` — set_status/get_status 实现
  - `references/feature-flags.md` — WRITE_GRAN × 6 编码差异

  **Acceptance Criteria**:
  - [ ] `cargo build` 编译通过
  - [ ] `cargo test --lib low_lvl` 状态表 UT 全部通过
  - [ ] WRAN=1 时 status table 编码与 C 版一致（对照 fdb_utils.c 注释）
  - [ ] GRAN=8/32/64 时同样正确

  **QA Scenarios**:
  ```
  Scenario: set_status GRAN=1 编码正确
    Tool: Bash
    Steps:
      1. `cargo test --lib low_lvl::tests::test_set_status_gran1` -- 调用 set_status(table, 4, 0), 1, 2, 3 分别验证
      2. 对照 C 版：index=0 → 全 FF；index=1 → 0x7F；index=2 → 0x3F；index=3 → 0x1F
    Expected Result: 每个编码值与 C 版完全一致
    Evidence: .sisyphus/evidence/task-4-set-status-gran1.log

  Scenario: get_status GRAN=1 解码正确
    Tool: Bash
    Steps:
      1. `cargo test --lib low_lvl::tests::test_get_status_gran1` -- 构造 status_table 分别测试 index 0-3
    Expected Result: 解码后的 index 与 C 版一致
    Evidence: .sisyphus/evidence/task-4-get-status-gran1.log

  Scenario: align_up / wg_align / wg_align_down 计算正确
    Tool: Bash
    Steps:
      1. `cargo test --lib low_lvl::tests::test_align_macros`
      2. 验证 FDB_ALIGN(13,4)=16, FDB_WG_ALIGN(13)=16 (GRAN=8), FDB_WG_ALIGN_DOWN(15)=16
    Expected Result: 所有对齐计算与 C 版一致
    Evidence: .sisyphus/evidence/task-4-align-tests.log
  ```

  **Evidence to Capture**:
  - [ ] task-4-set-status-gran1.log
  - [ ] task-4-get-status-gran1.log
  - [ ] task-4-align-tests.log

  **Commit**: NO (groups with Wave 1)

- [ ] 5. low_lvl.rs — flash I/O wrapper (read/erase/write/aligned write)

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
  - **Category**: `unspecified-high`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-A 循环+条件映射
    - `c-translate-to-rust`: §3 禁令表（无 unsafe 在非 FFI）

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-4)
  - **Blocks**: Tasks 6-9, 10+
  - **Blocked By**: Tasks 1, 2, 3, 4

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
    Evidence: .sisyphus/evidence/task-5-flash-io.log

  Scenario: continue_ff_addr 与 C 版一致
    Tool: Bash
    Steps:
      1. 在 MockFlash 写入数据：[0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF]
      2. 调用 continue_ff_addr(flash, 0, 9)
      3. C 版结果应为 wg_align(7) = 8 (GRAN=8)
    Expected Result: 返回地址与 C 版完全一致
    Evidence: .sisyphus/evidence/task-5-continue-ff.log
  ```

  **Evidence to Capture**:
  - [ ] task-5-flash-io.log
  - [ ] task-5-continue-ff.log

  **Commit**: YES (groups with Wave 1)
  - Message: `feat(scaffold): crate 骨架 + 类型定义 + FlashDevice trait + 底层 API + MockFlash`

- [ ] 6. CRC32 模块翻译 + UT

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
  - **Category**: `quick`
  - **Skills**: `c-translate-to-rust`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 7-9)
  - **Blocks**: Tasks 10, 11, 14+
  - **Blocked By**: Tasks 1, 2, 4

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
      1. `cargo test --lib crc32::tests::test_empty` → 0x00000000
      2. `cargo test --lib crc32::tests::test_hello` → 已知值
      3. `cargo test --lib crc32::tests::test_binary_ff` → 对比 C 版
    Expected Result: 所有 CRC 值与 C 版一致
    Evidence: .sisyphus/evidence/task-6-crc32.log
  ```

  **Evidence to Capture**:
  - [ ] task-6-crc32.log

  **Commit**: NO (groups with Wave 2)

- [ ] 7. write_status / read_status + 状态持久化 UT

  **What to do**:
  - 翻译 `fdb_utils.c:147-180` 中的 `_fdb_write_status` 和 `_fdb_read_status`
  - `_fdb_write_status` → `pub(crate) fn write_status(flash, addr, status_num, status_index, sync) -> Result<(), FdbErr>`
  - `_fdb_read_status` → `pub(crate) fn read_status(flash, addr, total_num) -> u32`
  - 依赖 Task 4 的 set_status/get_status 和 Task 5 的 flash I/O
  - 添加 UT：用 MockFlash 验证写入状态表后 read_back 解码正确
  - 添加 UT：验证状态表在不同 WRITE_GRAN 下的行为

  **Must NOT do**:
  - 不要将状态表改为 HashMap/BTreeMap

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `c-translate-to-rust`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6, 8-9)
  - **Blocks**: Tasks 10-16
  - **Blocked By**: Tasks 4, 5

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
    Evidence: .sisyphus/evidence/task-7-status-roundtrip.log
  ```

  **Evidence to Capture**:
  - [ ] task-7-status-roundtrip.log

  **Commit**: NO (groups with Wave 2)

- [ ] 8. blob API 翻译 + UT

  **What to do**:
  - 翻译 `fdb_def.h:335-344` 中 `struct fdb_blob`（已在 Task 2 完成）
  - 翻译 `fdb_utils.c:221-249` 中的 blob API：
    - `fdb_blob_make(blob, value_buf, buf_len)` → `pub fn blob_make(buf: &mut [u8]) -> Blob`
    - `fdb_blob_read(db, blob)` → `pub fn blob_read(flash: &mut F, blob: &mut Blob) -> usize`
  - Blob 是通用数据传输容器，在 KVDB/TSDB 中大量使用
  - 每个函数添加 `// c: fdb_utils.c:LINE` 注释
  - 添加 UT：验证 blob_make/blob_read 行为

  **Must NOT do**:
  - 不要将 Blob 改为 Cow<T>（保持简单）

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `c-translate-to-rust`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6-7, 9)
  - **Blocks**: Tasks 11, 15+
  - **Blocked By**: Tasks 4, 5

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
    Evidence: .sisyphus/evidence/task-8-blob.log
  ```

  **Evidence to Capture**:
  - [ ] task-8-blob.log

  **Commit**: NO (groups with Wave 2)

- [ ] 9. fdb.c — init/deinit/control 核心翻译 + UT

  **What to do**:
  - 翻译 `fdb.c:31-157` 中的核心初始化逻辑：
    - `_fdb_init_ex(db, name, path, type, user_data)` → `pub(crate) fn init_ex(db: &mut Db, name, path, flash, ...) -> Result<(), FdbErr>`
    - `_fdb_init_finish(db, result)` → `pub(crate) fn init_finish(db: &mut Db, result)`
    - `_fdb_deinit(db)` → `pub(crate) fn deinit(db: &mut Db)`
    - `_fdb_db_path(db)` → `pub(crate) fn db_path(db: &Db) -> &str`
  - FAL 部分（`if (!file_mode)`)翻译为 trait bound 版本
  - `log_is_show` static → `AtomicBool`（Skill §1-B global-state.md）
  - `FDB_ASSERT(db->sec_size != 0)` → `assert!` 宏
  - 每个函数添加 `// c: fdb.c:LINE` 注释
  - 添加 UT：验证 init_ex 的各种错误路径（分区不存在、大小不对齐、扇区不足）

  **Must NOT do**:
  - 不要保留 FAL 具体调用（fal_init/fal_partition_find 等）
  - 不要简化 sec_size 验证逻辑

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-A 宏→assert 映射
    - `c-translate-to-rust`: §1-B global-state.md（AtomicBool 替代 static mut）

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6-8)
  - **Blocks**: Tasks 10, 13, 16
  - **Blocked By**: Tasks 1, 2, 3, 4, 5

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb.c:31-157` — 完整 init/deinit 实现
  - `references/global-state.md` — static mut → AtomicBool 方案

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
    Evidence: .sisyphus/evidence/task-9-init-errors.log

  Scenario: init_ex 正常初始化
    Tool: Bash
    Steps:
      1. init_ex with name="test", sec_size=4096, max_size=16384 (4 sectors)
      2. 验证 db.init_ok = true
    Expected Result: 初始化成功，init_ok 为 true
    Evidence: .sisyphus/evidence/task-9-init-success.log
  ```

  **Evidence to Capture**:
  - [ ] task-9-init-errors.log
  - [ ] task-9-init-success.log

  **Commit**: YES (groups with Wave 2)
  - Message: `feat(core): CRC32 模块完成, 状态表操作完成, blob API 完成, init/deinit 核心完成`

- [ ] 10. kvdb.rs — KV sector header + KV header on-flash struct 翻译 + UT

  **What to do**:
  - 翻译 `fdb_kvdb.c:102-133` 中的 on-flash struct：
    - `struct sector_hdr_data` → `#[repr(C)] struct SectorHdrData` — 包含 status_table.store, status_table.dirty, magic, combined, reserved, padding
    - `struct kv_hdr_data` → `#[repr(C)] struct KvHdrData` — 包含 status_table, magic, len, crc32, name_len, value_len, padding
  - 翻译 `fdb_kvdb.c:34-99` 中的 KVDB 内部常量和宏：
    - `SECTOR_MAGIC_WORD=0x30424446`
    - `KV_MAGIC_WORD=0x3030564B`
    - `GC_MIN_EMPTY_SEC_NUM=1`
    - `FDB_SEC_REMAIN_THRESHOLD`
    - `SECTOR_STORE_OFFSET` / `SECTOR_DIRTY_OFFSET` → `core::mem::offset_of!` (参考 references/offsetof.md)
    - `KV_MAGIC_OFFSET` / `KV_LEN_OFFSET` 等 → `offset_of!`
    - `db_name` / `db_init_ok` / `db_sec_size` 等访问器宏 → Rust 方法
  - 添加 `assert_eq!(core::mem::size_of::<SectorHdrData>(), EXPECTED_SIZE)` 验证布局
  - 添加 `assert_eq!(core::mem::size_of::<KvHdrData>(), EXPECTED_SIZE)` 验证布局
  - 添加 UT：验证 SECTOR_STORE_OFFSET/KV_MAGIC_OFFSET 等与 C 版一致
  - 每个结构体添加 `// c: fdb_kvdb.c:LINE` 注释
  - 翻译 `fdb_kvdb.c:255-275` 中 KV 缓存操作（如启用 kv_cache feature）：
    - `update_sector_cache` / `get_sector_from_cache` / `update_kv_cache` / `get_kv_from_cache`

  **Must NOT do**:
  - 不要用 `transmute` 读/写 on-flash struct
  - 不要添加 PhantomData 到 on-flash struct
  - 不要将多个 struct 合并（SectorHdrData 和 KvHdrData 分开定义）
  - 不要改变 on-flash 字节布局

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B offsetof.md（offset_of! 用法）
    - `c-translate-to-rust`: §1-B on-flash-compat.md（布局验证）

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6-9 — 但 T10 与 T6-9 有依赖，Wave 2 末尾执行)
  - **Blocks**: Tasks 11, 12
  - **Blocked By**: Tasks 2, 4, 5, 6

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:34-133` — on-flash struct + 常量完整定义
  - `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:150-275` — KV 缓存操作函数
  - `references/offsetof.md` — offset_of! 翻译方案
  - `references/on-flash-compat.md` — size_of 布局验证
  - `references/feature-flags.md` — FDB_WRITE_GRAN padding 变体

  **Acceptance Criteria**:
  - [ ] ` cargo build` 编译通过（layout assertions 不触发）
  - [ ] `cargo test --lib kvdb::tests::test_header_layouts` 通过
  - [ ] offset_of 结果与 C 版 `offsetof` 一致

  **QA Scenarios**:
  ```
  Scenario: on-flash struct 布局验证
    Tool: Bash
    Steps:
      1. `cargo test --lib kvdb::tests::test_sector_hdr_size`
      2. `cargo test --lib kvdb::tests::test_kv_hdr_size`
      3. `cargo test --lib kvdb::tests::test_offsets` — 验证 STORE_OFFSET, KV_MAGIC_OFFSET 等
    Expected Result: 所有 size 和 offset 与 C 版完全一致
    Evidence: .sisyphus/evidence/task-10-layout-verify.log
  ```

  **Evidence to Capture**:
  - [ ] task-10-layout-verify.log

  **Commit**: YES (groups with Wave 2 end)
  - Message: `feat(kvdb): on-flash header structs (sector_hdr + kv_hdr) translated with layout verification`


- [ ] 11. KVDB 核心功能 1: read_kv, find_kv, get_kv 翻译 + UT

  **What to do**:
  - 翻译 `fdb_kvdb.c:280-346` `find_next_kv_addr` — 按 magic word 扫描下一个 KV
  - 翻译 `fdb_kvdb.c:312-346` `get_next_kv_addr` — 获取扇区内下一个 KV 地址
  - 翻译 `fdb_kvdb.c:348-414` `read_kv` — 读取 KV 节点（header + name + value + CRC 验证）
  - 翻译 `fdb_kvdb.c:416-502` `read_sector_info` — 读取扇区信息（header + 遍历 KV 计算 remain）
  - 翻译 `fdb_kvdb.c:504-526` `get_next_sector_addr` — 获取下一个扇区地址
  - 翻译 `fdb_kvdb.c:528-557` `kv_iterator` — 通用 KV 迭代器（带回调）
  - 翻译 `fdb_kvdb.c:559-607` `find_kv_cb` / `find_kv_no_cache` / `find_kv` — KV 查找（含缓存）
  - 翻译 `fdb_kvdb.c:609-644` `fdb_is_str` / `get_kv` — KV 读取
  - `void*` 回调参数 → 泛型闭包 `FnMut`（参考 references/void-callbacks.md）
  - 每个函数添加 `// c: fdb_kvdb.c:LINE` 注释
  - 添加 UT：构造 MockFlash 写入 KV 数据，验证 read_kv 能正确读取并 CRC 校验通过

  **Must NOT do**:
  - 不要将所有函数塞入主 struct impl
  - 不要简化 CRC 计算逻辑
  - 不要忽略 FDB_BIG_ENDIAN 条件（当前默认 Little Endian）

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B void-callbacks.md（回调泛型化）
    - `c-translate-to-rust`: §1-A for/while 循环映射

  **Parallelization**:
  - **Can Run In Parallel**: NO (依赖 T10)
  - **Parallel Group**: Wave 3 (with Tasks 12-13, 14-16)
  - **Blocks**: Tasks 12, 13
  - **Blocked By**: Tasks 6, 7, 8, 10

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:280-644` — KV 读取相关完整函数
  - `references/void-callbacks.md` — void* 回调 → FnMut 泛型闭包方案

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
    Evidence: .sisyphus/evidence/task-11-read-kv.log
  ```

  **Evidence to Capture**:
  - [ ] task-11-read-kv.log

  **Commit**: NO (groups with Wave 3)

- [ ] 12. KVDB 核心功能 2: write_kv, set_kv, del_kv, format_sector, update_sec_status 翻译 + UT

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
  - `fdb_kvdb_control` 的 SET_LOCK/SET_UNLOCK → trait bound 替代（参考 references/fn-ptr-cast.md）
  - 每个函数添加 `// c: fdb_kvdb.c:LINE` 注释
  - 添加 UT：完整 CRUD cycle（set→get→update→get→del→get→verify deleted）

  **Must NOT do**:
  - 不要在 on-flash struct 上添加 Rust 字段
  - 不要简化两阶段写入（PRE_WRITE → WRITE）逻辑
  - 不要合并 fdb_kv_set 和 fdb_kv_set_blob 为一个函数

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B goto-patterns.md（goto __exit → `?` + loop）
    - `c-translate-to-rust`: §1-B fn-ptr-cast.md（control 函数命令模式 → trait）
    - `c-translate-to-rust`: §3 禁令表

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on T11)
  - **Parallel Group**: Wave 3 (sequential after T11)
  - **Blocks**: Tasks 13, 18
  - **Blocked By**: Task 11

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:755-1430` — KV 写入+删除+移动完整实现
  - `references/goto-patterns.md` — goto __exit → `?` + RAII
  - `references/fn-ptr-cast.md` — 函数指针命令模式 → Builder

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
    Evidence: .sisyphus/evidence/task-12-crud.log

  Scenario: blob set/get
    Tool: Bash
    Steps:
      1. kv_set_blob("bin", [0x01..0x20], 32) → NoErr
      2. kv_get_blob("bin") → 返回 32 bytes，内容与写入一致
    Expected Result: blob 读写正确
    Evidence: .sisyphus/evidence/task-12-blob.log
  ```

  **Evidence to Capture**:
  - [ ] task-12-crud.log
  - [ ] task-12-blob.log

  **Commit**: NO (groups with Wave 3)

- [ ] 13. KVDB 核心功能 3: gc, iterator, set_default, print, init, check 翻译 + UT

  **What to do**:
  - 翻译 `fdb_kvdb.c:1098-1181` — GC 实现：
    - `gc_check_cb` / `do_gc` / `gc_collect_by_free_size` / `gc_collect`
  - 翻译 `fdb_kvdb.c:1386-1431` — `fdb_kv_set_default`
  - 翻译 `fdb_kvdb.c:1432-1507` — `fdb_kv_print`（输出日志可简化为 log::info!）
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

  **Must NOT do**:
  - 不要简化 GC 触发逻辑（空间耗尽 + alloc_kv 失败两种场景）
  - 不要忽略 recovery_check 标志
  - 不要改变 on-flash 格式

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B goto-patterns.md（__exit/__retry 翻译）
    - `c-translate-to-rust`: §1-B void-callbacks.md（回调泛型化）

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on T12)
  - **Parallel Group**: Wave 3 (sequential after T12)
  - **Blocks**: Tasks 20, 21
  - **Blocked By**: Task 12

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_kvdb.c:1098-1944` — GC + iterator + init + check 完整实现
  - `references/goto-patterns.md` — goto → loop / `?` 翻译

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
    Evidence: .sisyphus/evidence/task-13-gc.log

  Scenario: set_default 恢复
    Tool: Bash
    Steps:
      1. 写入多个自定义 KV
      2. kv_set_default(default_kvs)
      3. 验证所有自定义 KV 不可 get
      4. 验证所有默认 KV 可 get
    Expected Result: set_default 后只有默认 KV 存在
    Evidence: .sisyphus/evidence/task-13-default.log
  ```

  **Evidence to Capture**:
  - [ ] task-13-gc.log
  - [ ] task-13-default.log

  **Commit**: YES (groups with Wave 3)
  - Message: `feat(kvdb): KVDB 全功能实现完成（init, CRUD, iterator, GC, recovery, set_default）+ UT`

- [ ] 14. tsdb.rs — TSDB sector header + log index on-flash struct 翻译 + UT

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
  - 添加 `assert_eq!(core::mem::size_of::<...>(), EXPECTED)` 验证两个 on-flash struct 布局
  - 添加 UT：验证所有 offset 和 size 与 C 版一致
  - 每个结构体添加 `// c: fdb_tsdb.c:LINE` 注释

  **Must NOT do**:
  - 不要添加 Rust 字段到 on-flash struct
  - 不要用 transmute 读/写
  - 不要将 sector_hdr 和 log_idx 合并到同一 struct

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B offsetof.md
    - `c-translate-to-rust`: §1-B on-flash-compat.md
    - `c-translate-to-rust`: §1-B conditional-struct-layout.md

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 11-13, 15-16)
  - **Blocks**: Tasks 15, 16
  - **Blocked By**: Tasks 2, 4, 5, 6

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:28-133` — TSDB on-flash struct + 常量
  - `references/offsetof.md` — offset_of! 翻译
  - `references/conditional-struct-layout.md` — timestamp_64bit padding

  **Acceptance Criteria**:
  - [ ] `cargo build` 编译通过（layout 验证不触发）
  - [ ] `cargo test --lib tsdb::tests::test_header_layouts`
  - [ ] offset_of 值与 C 版一致

  **QA Scenarios**:
  ```
  Scenario: TSDB on-flash struct size 验证
    Tool: Bash
    Steps:
      1. `cargo test --lib tsdb::tests::test_tsdb_sector_hdr_size`
      2. `cargo test --lib tsdb::tests::test_log_idx_size`
      3. `cargo test --lib tsdb::tests::test_tsdb_offsets`
    Expected Result: 所有 size 和 offset 与 C 版一致
    Evidence: .sisyphus/evidence/task-14-tsdb-layout.log
  ```

  **Evidence to Capture**:
  - [ ] task-14-tsdb-layout.log

  **Commit**: NO (groups with Wave 3 end)

- [ ] 15. TSDB 核心功能 1: read_tsl, format_sector, write_tsl, update_sec_status 翻译 + UT

  **What to do**:
  - 翻译 `fdb_tsdb.c:147-175` — `read_tsl` 读取 TSL 节点
  - 翻译 `fdb_tsdb.c:177-240` — 扇区/TSL 地址操作：
    - `get_next_sector_addr` / `get_next_tsl_addr` / `get_last_tsl_addr` / `get_last_sector_addr`
  - 翻译 `fdb_tsdb.c:242-327` — 扇区读取与格式化：
    - `read_sector_info` / `format_sector`
  - 翻译 `fdb_tsdb.c:350-499` — TSL 写入：
    - `write_tsl` / `update_sec_status` / `tsl_append`
  - _FDB_WRITE_STATUS / FLASH_WRITE 宏 → 辅助 fn 返回 Result（参考 references/macro-to-fn.md）
  - `goto __exit` → `?` + RAII
  - 公共 API `fdb_tsl_append` / `fdb_tsl_append_with_ts`
  - 每个函数添加 `// c: fdb_tsdb.c:LINE` 注释
  - 添加 UT：写入 TSL 后 read_tsl 验证数据一致；append 后时间戳递增验证；扇区满后自动切换

  **Must NOT do**:
  - 不要改变 TSL 索引布局
  - 不要忽略 PRE_WRITE → WRITE 两阶段状态转换
  - 不要简化 update_sec_status 中的 end_info 保存逻辑

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B macro-to-fn.md（do{...}while(0) 宏 → fn + `?`）
    - `c-translate-to-rust`: §1-B goto-patterns.md

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on T14)
  - **Parallel Group**: Wave 3 (after T14)
  - **Blocks**: Tasks 16, 17
  - **Blocked By**: Tasks 7, 8, 14

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:147-499` — TSDB 核心读写扇区操作
  - `references/macro-to-fn.md` — 宏函数 → fn + Result
  - `references/goto-patterns.md` — goto 翻译

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
    Evidence: .sisyphus/evidence/task-15-append-read.log
  ```

  **Evidence to Capture**:
  - [ ] task-15-append-read.log

  **Commit**: NO (groups with Wave 3)

- [ ] 16. TSDB 核心功能 2: init, deinit, control, recovery, clean 翻译 + UT

  **What to do**:
  - 翻译 `fdb_tsdb.c:866-915` — TSDB 初始化辅助：
    - `check_sec_hdr_cb` → fn + 状态收集 struct
    - `format_all_cb` / `tsl_format_all`
  - 翻译 `fdb_tsdb.c:924-929` — `fdb_tsl_clean`
  - 翻译 `fdb_tsdb.c:938-1003` — `fdb_tsdb_control`（改为 Builder）
  - 翻译 `fdb_tsdb.c:1018-1116` — `fdb_tsdb_init` / `fdb_tsdb_deinit`
  - control 命令模式 → Builder 模式 + type-safe setter（参考 references/fn-ptr-cast.md）
  - lock/unlock → trait bound 替代 void* 函数指针
  - 每个函数添加 `// c: fdb_tsdb.c:LINE` 注释
  - 添加 UT：init 成功；rollover=true 时环形回绕；rollover=false 时空间耗尽返回 SAVED_FULL；max_len >= sec_size 断言失败

  **Must NOT do**:
  - 不要保留 void* 函数指针 lock/unlock
  - 不要保留 control 命令模式 switch

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B fn-ptr-cast.md（control → Builder）
    - `c-translate-to-rust`: §1-B goto-patterns.md

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on T9, T15)
  - **Parallel Group**: Wave 3 (after T9 and T15)
  - **Blocks**: Tasks 17, 18, 22
  - **Blocked By**: Tasks 9, 15

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:866-1116` — TSDB init/control/clean
  - `references/fn-ptr-cast.md` — 命令模式 → Builder + setter

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
    Evidence: .sisyphus/evidence/task-16-rollover.log
  ```

  **Evidence to Capture**:
  - [ ] task-16-rollover.log

  **Commit**: YES (groups with Wave 3 end)
  - Message: `feat(tsdb): TSDB 核心函数完成（init, append, recovery, clean）+ UT`


- [ ] 17. TSDB 核心功能 3: iter, iter_reverse, iter_by_time, query_count, set_status, max_blob_count 翻译 + UT

  **What to do**:
  - 翻译 `fdb_tsdb.c:556-597` — `fdb_tsl_iter` 正序遍历
  - 翻译 `fdb_tsdb.c:606-649` — `fdb_tsl_iter_reverse` 逆序遍历
  - 翻译 `fdb_tsdb.c:654-768` — `search_start_tsl_addr`（二分查找起始 TSL）+ `fdb_tsl_iter_by_time` 范围查询
  - 翻译 `fdb_tsdb.c:771-805` — `query_count_cb` + `fdb_tsl_query_count`
  - 翻译 `fdb_tsdb.c:807-827` — `fdb_tsl_max_blob_count`
  - 翻译 `fdb_tsdb.c:838-864` — `fdb_tsl_set_status` + `fdb_tsl_to_blob`
  - 回调 `bool(cb)(tsl, arg)` → `impl FnMut(&Tsl) -> bool`（参考 references/void-callbacks.md）
  - 每个函数添加 `// c: fdb_tsdb.c:LINE` 注释
  - 添加 UT：验证正序/逆序/范围查询的正确调用顺序；验证 query_count 统计；验证 set_status 后重读状态一致

  **Must NOT do**:
  - 不要简化二分查找逻辑
  - 不要改变 from>to 表示反向遍历的语义

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `c-translate-to-rust`
    - `c-translate-to-rust`: §1-B void-callbacks.md
    - `c-translate-to-rust`: §1-B goto-patterns.md

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on T15, T16)
  - **Parallel Group**: Wave 4 (with Tasks 18-20)
  - **Blocks**: Tasks 22, 23, 24
  - **Blocked By**: Tasks 15, 16

  **References**:
  - `D:\MyCode\temp\FlashDB\src\fdb_tsdb.c:556-864` — TSDB 遍历与查询完整实现
  - `references/void-callbacks.md` — 回调泛型化

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
    Evidence: .sisyphus/evidence/task-17-iter.log

  Scenario: 范围查询 [200, 400] 返回 3 条
    Tool: Bash
    Steps:
      1. iter_by_time(200, 400)
      2. 验证回调被调用 3 次，时间戳为 200, 300, 400
    Expected Result: 范围查询正确
    Evidence: .sisyphus/evidence/task-17-range.log
  ```

  **Evidence to Capture**:
  - [ ] task-17-iter.log
  - [ ] task-17-range.log

  **Commit**: NO (groups with Wave 4)

- [ ] 18. TSDB 核心功能 4: 综合验证（BDD 前置）+ 补充 UT

  **What to do**:
  - 补充之前未覆盖的边缘 case UT：
    - 多扇区 rollover
    - PRE_WRITE 恢复
    - 扇区切换时 end_info 保存
    - FIXED_BLOB_SIZE 模式
    - `fdb_tsl_max_blob_count` 边界
  - 验证所有 TSDB 公共 API 完整覆盖（对照 flashdb.h:62-71）
  - 运行 `cargo test --lib tsdb` 确保全部通过

  **Must NOT do**:
  - 不要添加不存在的功能

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `c-translate-to-rust`

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on T12, T16, T17)
  - **Parallel Group**: Wave 4 (after T17)
  - **Blocks**: Tasks 22, 23, 24
  - **Blocked By**: Tasks 12, 16, 17

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
      1. `cargo test --lib tsdb` → 全部通过，无 failures
      2. `cargo test --lib tsdb -- --list` → 列出所有测试，与 KVDB 测试数量相当
    Expected Result: 全部 UT 通过
    Evidence: .sisyphus/evidence/task-18-tsdb-full-suite.log
  ```

  **Evidence to Capture**:
  - [ ] task-18-tsdb-full-suite.log

  **Commit**: YES (groups with Wave 4)
  - Message: `feat(tsdb): TSDB 全功能实现完成（iter, range query, count, status, max_blob_count）+ 完整 UT`

- [ ] 19. BDD cucumber-rust 集成框架搭建

  **What to do**:
  - 添加 BDD 测试 infrastructure：
    - `Cargo.toml` 添加 dev-dependencies: `cucumber = "0.21"` + `tokio`
    - `features/` 目录复制现有 6 个 .feature 文件（中文 Gherkin）
    - `tests/bdd/` 目录创建 step definition 文件
    - 创建 `tests/bdd/mod.rs` — cucumber World 定义（包含 FlashDB 实例 + MockFlash）
    - 注册所有 6 个 feature 文件的 scenario
  - World struct 实现：
    - `struct FlashWorld { flash: MockFlash, kvdb: FdbKvdb, tsdb: FdbTsdb, ... }`
    - 每个 feature 文件对应的 step 模块

  **Must NOT do**:
  - 不要修改现有 .feature 文件内容
  - 不要跳过任何 scenario

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with Tasks 17-18, then T20-21)
  - **Blocks**: Tasks 20-24
  - **Blocked By**: Task 1 (Cargo.toml)

  **References**:
  - `D:\MyCode\temp\tmp-FlashDB\.opencode\harness\features\` — 6 个 Gherkin feature 文件

  **Acceptance Criteria**:
  - [ ] `cargo test --test bdd` 编译通过（step 可暂时 unimplemented）
  - [ ] `features/` 目录有 6 个 .feature 文件

  **QA Scenarios**:
  ```
  Scenario: BDD 框架编译通过
    Tool: Bash
    Steps:
      1. `cargo build --tests`
      2. 检查 features/ 目录有 6 个 .feature 文件
    Expected Result: 编译成功，feature 文件完整
    Evidence: .sisyphus/evidence/task-19-bdd-framework.log
  ```

  **Evidence to Capture**:
  - [ ] task-19-bdd-framework.log

  **Commit**: NO (groups with Wave 4 end)

- [ ] 20. BDD 场景实现: KVDB init + CRUD (2 feature files)

  **What to do**:
  - 实现 `features/kvdb-init.feature` 全部 10 个 scenario
  - 实现 `features/kvdb-crud.feature` 全部 10 个 scenario
  - 每个 scenario 的 Given/When/Then step 绑定真实 FlashDB API 调用
  - Flash 后端使用 MockFlash
  - 断言必须有真实验证（禁止虚假断言）

  **Must NOT do**:
  - 不要修改 feature 文件内容
  - 不要使用 fake assertions（如 `assert!(true)`）

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 21)
  - **Parallel Group**: Wave 4-5
  - **Blocks**: Final wave
  - **Blocked By**: Tasks 13, 19

  **References**:
  - `D:\MyCode\temp\tmp-FlashDB\.opencode\harness\features\kvdb-init.feature` — KVDB init BDD
  - `D:\MyCode\temp\tmp-FlashDB\.opencode\harness\features\kvdb-crud.feature` — KVDB CRUD BDD

  **Acceptance Criteria**:
  - [ ] `cargo test --test bdd::kvdb_init` 全部 scenario 通过
  - [ ] `cargo test --test bdd::kvdb_crud` 全部 scenario 通过

  **QA Scenarios**:
  ```
  Scenario: KVDB init feature 全部通过
    Tool: Bash
    Steps:
      1. `cargo test --test bdd -- features/kvdb-init.feature`
    Expected Result: 10 scenarios 全部 PASS
    Evidence: .sisyphus/evidence/task-20-kvdb-init-bdd.log
  ```

  **Evidence to Capture**:
  - [ ] task-20-kvdb-init-bdd.log
  - [ ] task-20-kvdb-crud-bdd.log

  **Commit**: NO (groups with Wave 4-5 end)

- [ ] 21. BDD 场景实现: KVDB iteration + GC (1 feature file)

  **What to do**:
  - 实现 `features/kvdb-iteration-gc.feature` 全部 8 个 scenario
  - 每个 scenario 绑定真实 API 调用
  - 断言必须有真实验证

  **Must NOT do**:
  - 不要修改 feature 文件
  - 不要虚假断言

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 20)
  - **Parallel Group**: Wave 4-5
  - **Blocks**: Final wave
  - **Blocked By**: Tasks 13, 19

  **References**:
  - `D:\MyCode\temp\tmp-FlashDB\.opencode\harness\features\kvdb-iteration-gc.feature`

  **Acceptance Criteria**:
  - [ ] `cargo test --test bdd -- features/kvdb-iteration-gc.feature` 全部通过

  **QA Scenarios**:
  ```
  Scenario: KVDB iteration + GC feature 全部通过
    Tool: Bash
    Steps:
      1. `cargo test --test bdd -- features/kvdb-iteration-gc.feature`
    Expected Result: 8 scenarios 全部 PASS
    Evidence: .sisyphus/evidence/task-21-kvdb-iter-gc.log
  ```

  **Evidence to Capture**:
  - [ ] task-21-kvdb-iter-gc.log

  **Commit**: NO (groups with Wave 4-5 end)

- [ ] 22. BDD 场景实现: TSDB init (1 feature file)

  **What to do**:
  - 实现 `features/tsdb-init.feature` 全部 11 个 scenario
  - 绑定真实 API 调用
  - 断言必须有真实验证

  **Must NOT do**:
  - 不要修改 feature 文件
  - 不要虚假断言

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 23, 24)
  - **Parallel Group**: Wave 5
  - **Blocks**: Final wave
  - **Blocked By**: Tasks 16, 18, 19

  **References**:
  - `D:\MyCode\temp\tmp-FlashDB\.opencode\harness\features\tsdb-init.feature`

  **Acceptance Criteria**:
  - [ ] `cargo test --test bdd -- features/tsdb-init.feature` 全部通过

  **QA Scenarios**:
  ```
  Scenario: TSDB init feature 全部通过
    Tool: Bash
    Steps:
      1. `cargo test --test bdd -- features/tsdb-init.feature`
    Expected Result: 11 scenarios 全部 PASS
    Evidence: .sisyphus/evidence/task-22-tsdb-init.log
  ```

  **Evidence to Capture**:
  - [ ] task-22-tsdb-init.log

  **Commit**: NO (groups with Wave 4-5 end)

- [ ] 23. BDD 场景实现: TSDB append (1 feature file)

  **What to do**:
  - 实现 `features/tsdb-append.feature` 全部 13 个 scenario
  - 绑定真实 API 调用
  - 断言必须有真实验证

  **Must NOT do**:
  - 不要修改 feature 文件
  - 不要虚假断言

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 22, 24)
  - **Parallel Group**: Wave 5
  - **Blocks**: Final wave
  - **Blocked By**: Tasks 16, 18, 19

  **References**:
  - `D:\MyCode\temp\tmp-FlashDB\.opencode\harness\features\tsdb-append.feature`

  **Acceptance Criteria**:
  - [ ] `cargo test --test bdd -- features/tsdb-append.feature` 全部通过

  **QA Scenarios**:
  ```
  Scenario: TSDB append feature 全部通过
    Tool: Bash
    Steps:
      1. `cargo test --test bdd -- features/tsdb-append.feature`
    Expected Result: 13 scenarios 全部 PASS
    Evidence: .sisyphus/evidence/task-23-tsdb-append.log
  ```

  **Evidence to Capture**:
  - [ ] task-23-tsdb-append.log

  **Commit**: NO (groups with Wave 4-5 end)

- [ ] 24. BDD 场景实现: TSDB query + management (1 feature file)

  **What to do**:
  - 实现 `features/tsdb-query-management.feature` 全部 14 个 scenario
  - 绑定真实 API 调用
  - 断言必须有真实验证

  **Must NOT do**:
  - 不要修改 feature 文件
  - 不要虚假断言

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 22, 23)
  - **Parallel Group**: Wave 5
  - **Blocks**: Final wave
  - **Blocked By**: Tasks 17, 18, 19

  **References**:
  - `D:\MyCode\temp\tmp-FlashDB\.opencode\harness\features\tsdb-query-management.feature`

  **Acceptance Criteria**:
  - [ ] `cargo test --test bdd -- features/tsdb-query-management.feature` 全部通过

  **QA Scenarios**:
  ```
  Scenario: TSDB query + management feature 全部通过
    Tool: Bash
    Steps:
      1. `cargo test --test bdd -- features/tsdb-query-management.feature`
    Expected Result: 14 scenarios 全部 PASS
    Evidence: .sisyphus/evidence/task-24-tsdb-query.log
  ```

  **Evidence to Capture**:
  - [ ] task-24-tsdb-query.log

  **Commit**: YES (groups with Wave 5)
  - Message: `test(bdd): 所有 BDD 场景完成（6 feature files, 66 scenarios）+ cucumber-rust 集成`


---

## Final Verification Wave (MANDATORY)

> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, cargo test output). For each "Must NOT Have": search codebase for forbidden patterns. Check evidence files exist. Compare deliverables against plan.
  特别检查：所有 on-flash struct 有 `#[repr(C)]` + size_of 验证；所有 fn 有 `// c: xxx.c:LINE` 注释；FAL 和 RT-Thread 集成层未实现；BDD feature 文件与 6 个原始 .feature 文件内容一致。
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo clippy -- -D warnings` + `cargo test`. Check: `unwrap()`/`expect()` prohibited; `unsafe` outside FFI; `_` prefix variables; `unimplemented!()`/`todo!()` placeholders; fake assertions; AI slop.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real Manual QA** — `unspecified-high`
  Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Run `cargo test` and `cargo test --test bdd`. Verify MockFlash NOR flash semantics. Verify CRC32 matches C. Verify on-flash struct sizes match C.
  Save to `.sisyphus/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff. Verify 1:1 — everything in spec built (no missing), nothing beyond spec built (no creep). Check "Must NOT do" compliance.
  特别检查：C 源码每个公共函数有对应 Rust fn（不遗漏）；未擅自添加新功能；BDD step 与 feature 文件 scenario 1:1 对应；测试数据对齐。
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`


---

## Commit Strategy

- Wave 1: `feat(scaffold): init rust crate with Cargo.toml, def.rs 类型定义，low_lvl.rs 底层抽象，mock_flash.rs FlashDevice trait + 内存模拟实现`
- Wave 2: `feat(core): CRC32 模块，状态表操作，blob API，init/deinit 核心 + UT`
- Wave 3: `feat(kvdb): KVDB 全功能实现（init, CRUD, iterator, GC, recovery）+ UT`
- Wave 3: `feat(tsdb): TSDB 全功能实现（init, append, iter, query, clean）+ UT`
- Wave 4-5: `test(bdd): cucumber-rust 集成 + 6 feature files 全场景覆盖`

---

## Success Criteria

### Verification Commands
```bash
cargo build          # Expected: success, zero warnings
cargo test           # Expected: all tests pass
cargo clippy -- -D warnings  # Expected: zero clippy warnings
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All on-flash structs size_of verified
- [ ] All UTs pass with real assertions
- [ ] BDD scenarios cover all 6 feature files
- [ ] Zero unsafe outside FFI boundary
- [ ] 1:1 映射 — 每个 C 函数有对应 Rust fn

---