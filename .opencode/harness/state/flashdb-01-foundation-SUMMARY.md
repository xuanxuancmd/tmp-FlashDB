# 01-foundation 编码完成

## 完成情况

| task_id | 描述 | commit | 状态 |
|---------|------|--------|------|
| T1 | Cargo.toml scaffolding + crate 配置 (lib.rs) | 4beb567 | ✅ 完成（上一会话已就绪，本次纳入首次提交） |
| T2 | def.rs — 所有 enum/struct/类型定义翻译 | 4beb567 | ✅ 完成（修复 `From<u32> for i32` 与 `FdbDefaultKv` Debug） |
| T3 | FlashDevice trait + MockFlash 内存模拟 | 4beb567 | ✅ 完成（修复 lib.rs `FlashError` 与 mock_flash `FdbByte` 失效导入） |
| T4 | low_lvl.rs — 常量宏 + 对齐宏 + 状态表 set/get | 4beb567 | ✅ 完成 |
| T5 | low_lvl.rs — flash I/O wrapper + continue_ff_addr | 4beb567 | ✅ 完成 |
| T6 | CRC32 模块（256 项查找表 + calc_crc32） | 4beb567 | ✅ 完成 |
| T7 | write_status / read_status + 状态持久化 | 4beb567 | ✅ 完成 |
| T8 | blob API（blob_make / blob_read） | 4beb567 | ✅ 完成 |
| T9 | init/deinit/control 核心翻译 + C 等价性集成测试 | 8259c7a | ✅ 完成 |

## 构建/测试自检

- `cargo build`: **pass**（零 warning，no_std 兼容）
- `cargo test`:
  - lib 单元测试: **42 passed / 0 failed**
  - `foundation_equiv` 集成测试: **9 passed / 0 failed**
  - doc-tests: 0
- feature 矩阵编译自检（T1 验收）:
  - `cargo build` (default kvdb+tsdb, GRAN=1): pass
  - `cargo build --no-default-features`: pass
  - `cargo build --features "kvdb,gran_8"`: pass
  - `cargo build --features "tsdb"`: pass
  - `cargo build --features "kvdb,gran_64"`: pass

## 关键实现说明

### C→Rust 翻译决策（依据 c-translate-to-rust skill）
- **FlashDevice trait**：C 的 FAL vtable（`fal_partition_read/write/erase`）+ `db->file_mode` 分发 → Rust `trait FlashDevice { read/write/erase }`，flash I/O wrapper 直接转发 trait 调用（参考 `references/fal-to-trait.md`）。
- **C 继承** `struct fdb_kvdb { struct fdb_db parent; }` → 组合 + `AsRef<FdbDb>`（禁用 `Deref`，参考 `references/c-inheritance.md`）。
- **`FDB_WRITE_GRAN` 条件编译**：沿用 T1/T2 已确立的 `pub const FDB_WRITE_GRAN: u32`（Cargo feature 选择）+ `if FDB_WRITE_GRAN == 1` 常量折叠分支（编译期消歧），与 def.rs `status_table_size_const` 模式一致。
- **`static bool log_is_show`** → `core::sync::atomic::AtomicBool`（no_std 可用，禁用 `static mut`）。
- **`fdb_time_t`**：feature `timestamp_64bit` 选择 i32/i64；`FDB_DATA_UNUSED as FdbTime` 显式转换（Rust 无隐式整型提升）。
- **`sync` 参数**：按 Plan T5 must-not-do 移除（trait write 同步）。
- **`_fdb_read_status` 忽略读错误**：1:1 还原 C 行为（`let _ = flash_read(...)`）。

### on-flash 布局验证
- `KvCacheNode`：`const _: () = assert!(size_of::<KvCacheNode>() == 8)` 编译期通过。
- 其余 on-flash struct（SectorHdrData/KvHdrData 等）属于 Plan 2/3 范围，本 Plan 不涉及。

### C 等价性金标准（`tests/foundation_equiv.rs`）
- CRC32 空输入 = 0x00000000；"123456789" = 0xCBF43926（标准 CRC-32/ISO-HDLC 校验值，与 C `fdb_utils.c:21-89` 表一致）。
- 状态表编码（GRAN=1, num=4）：index 0→0xFF, 1→0x7F, 2→0x3F, 3→0x1F（对照 `fdb_utils.c:91-107` 注释表）。
- blob 往返 + saved_len 截断。

### Plan QA 场景勘误
Plan T4/T5 的 QA 场景中 `FDB_WG_ALIGN(13)=16 (GRAN=8)` 与 `wg_align(7)=8 (GRAN=8)` 不符合 C 宏语义（GRAN 单位为 bit：GRAN=8 → 字节对齐，wg_align(13)=13）。本实现严格 1:1 对照 C 源码：GRAN=1 时 wg_align(13)=13、continue_ff_addr 返回 7；GRAN=64 时（`gran_64` feature）才出现 16/8 的对齐与 padding，由 `#[cfg(feature = "gran_64")]` 测试覆盖。

## 提交结构说明（断点续传）
本次为断点续传：T2-T9 在同一会话内完成，且 `lib.rs`（T1 产物）已前向声明所有模块的 `pub use` 重导出，导致任意可编译提交必须同时包含 `low_lvl.rs` 与 `init.rs`。故两次提交按文件可编译边界划分：
- **4beb567 (Wave 1)**：全部源码（Cargo.toml + src/*）— crate 骨架 + 类型 + trait + 底层 API + MockFlash + CRC32 + 状态表 + blob + init/deinit。
- **8259c7a (Wave 2)**：`tests/foundation_equiv.rs` C 等价性集成测试。

## 出口条件核对
- [x] `cargo build` 编译通过，零 warning
- [x] `cargo test --lib` 全部 UT 通过（42）
- [x] 所有 on-flash struct size_of 编译期断言通过（KvCacheNode==8）
- [x] Foundation 公共 API 全部可被 Plan 2/3 引用（lib.rs `pub use` 重导出）
- [x] `cargo test --test foundation_equiv` 通过（9）
- [x] 两次 Commit 完成（4beb567 + 8259c7a）
