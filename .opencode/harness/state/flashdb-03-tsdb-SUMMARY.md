# 03-tsdb 编码完成

## 完成情况

| task_id | 描述 | commit | 状态 |
|---------|------|--------|------|
| t-14 | tsdb.rs — TSDB sector header + log index on-flash struct 翻译 + 常量/宏 + UT | e610346 | ✅ 完成 |
| t-15 | read_tsl, get_next_sector_addr, get_next_tsl_addr, get_last_tsl_addr, get_last_sector_addr, read_sector_info, format_sector, write_tsl, update_sec_status, tsl_append, fdb_tsl_append, fdb_tsl_append_with_ts 翻译 + UT | 2886dc6 | ✅ 完成 |
| t-16 | check_sec_hdr_cb, format_all_cb, tsl_format_all, fdb_tsl_clean, fdb_tsdb_control (Builder), fdb_tsdb_init, fdb_tsdb_deinit 翻译 + UT | 2886dc6 | ✅ 完成 |
| t-17 | fdb_tsl_iter, fdb_tsl_iter_reverse, search_start_tsl_addr, fdb_tsl_iter_by_time, query_count_cb, fdb_tsl_query_count, fdb_tsl_max_blob_count, fdb_tsl_set_status, fdb_tsl_to_blob 翻译 + UT | 8f97bac | ✅ 完成 |
| t-18 | 综合验证 + 补充 UT（多扇区 rollover, PRE_WRITE 恢复, end_info 保存, max_blob_count 边界, reboot 持久性, clean after reboot）+ C 等价性测试 | 8f97bac | ✅ 完成 |

## 构建/测试自检

- `cargo check`: **pass**（最后一次）
- `cargo test`: **pass**（最后一次）
  - 93 单元测试（42 Foundation + 51 TSDB）
  - 9 Foundation 等价性集成测试
  - 8 TSDB C 等价性集成测试
  - 总计 110 个测试全部通过

## 实现概要

### 文件清单
- `src/tsdb.rs` — TSDB 完整实现（fdb_tsdb.c 1118 行 1:1 翻译），含 51 个单元测试
- `src/lib.rs` — 添加 `mod tsdb;` 声明 + `FdbGetTime` 重导出
- `Cargo.toml` — 添加 `fixed_blob_size` feature + `[[test]]` 配置
- `tests/c-port/tsdb_equiv.rs` — C 等价性集成测试（8 个测试，迁移自 fdb_tsdb_tc.c）

### 公共 API 覆盖（fdb_tsdb.c → Rust 1:1 映射）
| C 函数 | Rust 方法 |
|--------|----------|
| fdb_tsdb_init | FdbTsdb::init |
| fdb_tsdb_deinit | FdbTsdb::deinit |
| fdb_tsdb_control | Builder 模式 (set_sec_size, set_rollover, set_lock, etc.) |
| fdb_tsl_append | FdbTsdb::tsl_append |
| fdb_tsl_append_with_ts | FdbTsdb::tsl_append_with_ts |
| fdb_tsl_iter | FdbTsdb::tsl_iter |
| fdb_tsl_iter_reverse | FdbTsdb::tsl_iter_reverse |
| fdb_tsl_iter_by_time | FdbTsdb::tsl_iter_by_time |
| fdb_tsl_query_count | FdbTsdb::tsl_query_count |
| fdb_tsl_max_blob_count | FdbTsdb::tsl_max_blob_count |
| fdb_tsl_set_status | FdbTsdb::tsl_set_status |
| fdb_tsl_clean | FdbTsdb::tsl_clean |
| fdb_tsl_to_blob | FdbTsdb::tsl_to_blob |

### On-flash struct 布局验证
- `TsdbSectorHdrData` — `#[repr(C)]` + `size_of` 编译期断言
- `LogIdxData` — `#[repr(C)]` + `size_of` 编译期断言（支持 `fixed_blob_size` feature 变体）
- `TsdbSectorEndInfo` — `#[repr(C)]` + `size_of` 编译期断言
- 所有 offset 常量使用 `core::mem::offset_of!` 从 struct 定义派生

### 关键设计决策
1. **Flash 设备分离**：C 的 `db->storage` union → Rust `FlashDevice` trait，每个 I/O 方法接收 `&F`/`&mut F` 参数
2. **`goto __exit` → `?` + early return**：所有 C 的 goto 模式重构为 Rust 的 `?` 操作符和提前返回
3. **回调 → 泛型闭包**：`bool(*cb)(tsl, arg)` → `impl FnMut(&FdbTsl) -> bool`
4. **control 命令 → Builder 模式**：`fdb_tsdb_control(db, cmd, arg)` switch → 类型安全的 setter/getter 方法
5. **lock/unlock**：保留 `Option<fn(&mut FdbDb)>`，iter 函数使用 `&self`（调用者负责同步）
6. **序列化**：不使用 `transmute`，通过 offset 常量 + `read_u32_ne`/`write_time_ne` 安全读写
