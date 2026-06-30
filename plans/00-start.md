# FlashDB C→Rust 翻译迁移 — Start 文档

> 本文档是 FlashDB C→Rust 翻译项目的入口与总览。所有子 Plan 与 Final Plan 引用本文档作为上下文根。
>
---

## 1. 需求总目标

将开源 FlashDB（原代码路径从上下文中提取）1:1 翻译为 Rust 嵌入式库，保持 on-flash 格式字节级兼容，配套单元测试（UT）、C 等价性测试和 BDD 验收测试（FT）。

**核心交付物**：
- 完整 Rust `flashdb` crate（`no_std` 兼容）
- `FlashDevice` trait + `MockFlash` 内存模拟实现
- KVDB 全功能：init, CRUD, iterator, GC, recovery
- TSDB 全功能：init, append, iter/iter_reverse/iter_by_time, query, clean, status
- CRC32 工具模块（与 C 版字节级一致）
- UT（禁止虚假断言）+ C 等价性测试 + BDD Gherkin 测试（6 个 `.feature` 文件）


## 2. 多Plan

### 2.1 拆分规则应用

| 规则 | 应用 |
|------|------|
| 单 Plan ≤ 2000 行 | 原 Plan 1657 行内容拆为 4 个 Plan，每个约 200-700 行 |
| 重构/迁移类按目录拆 | C 源码 5 文件自然分组：`fdb.c`+`fdb_utils.c`+`inc/` = 共享基础设施；`fdb_kvdb.c` = KVDB；`fdb_tsdb.c` = TSDB；`fdb_file.c` = 文件模式（暂缓） |
| 循环依赖合并 | 无循环依赖，Foundation 是唯一共享根 |
| 高内聚优先 | KVDB 与 TSDB 是独立业务场景，分别成 Plan |
| 共享类型前置单独立 Plan | Foundation Plan 包含所有共享类型与底层 API，先于 KVDB/TSDB 执行 |

### 2.2 Plan 列表

| Plan | 文件 | 范围 | 原 Plan Tasks | 预估 C 源码行数 |
|------|------|------|---------------|----------------|
| **Plan 1** Foundation | `01-foundation.md` | Cargo.toml + def.rs + low_lvl.rs + flash_trait.rs + mock_flash.rs + fdb.c init | T1-T9 | ~506L (fdb.c 157 + fdb_utils.c 349) |
| **Plan 2** KVDB | `02-kvdb.md` | kvdb.rs 全功能实现 + UT | T10-T13 | 1944L (fdb_kvdb.c) |
| **Plan 3** TSDB | `03-tsdb.md` | tsdb.rs 全功能实现 + UT | T14-T18 | 1118L (fdb_tsdb.c) |
| **Final** Integration & BDD | `04-final.md` | 集成编译 + BDD Step Def + 验证 | T19-T24 + F1-F4 | — |

---

## 3. Plan 依赖 DAG

```
                    ┌─────────────────────┐
                    │  Plan 1 Foundation  │
                    │   (T1-T9, 必须先)   │
                    └──────────┬──────────┘
                               │
                ┌──────────────┴──────────────┐
                ▼                             ▼
      ┌──────────────────┐          ┌──────────────────┐
      │  Plan 2 KVDB     │          │  Plan 3 TSDB     │
      │   (T10-T13)      │          │   (T14-T18)      │
      └────────┬─────────┘          └────────┬─────────┘
               │                             │
               └──────────────┬──────────────┘
                              ▼
                  ┌────────────────────────┐
                  │  Final Plan            │
                  │  (T19-T24 + F1-F4)     │
                  │  集成+BDD+验证         │
                  └────────────────────────┘
```

### 3.1 执行顺序与并发

| 阶段 | 执行 Plan | 并发度 | 备注 |
|------|----------|--------|------|
| 阶段 1 | Plan 1 Foundation | 1 | 串行，所有后续 Plan 依赖此 Plan 产物 |
| 阶段 2 | Plan 2 KVDB ‖ Plan 3 TSDB | **2**（可并发） | 两者互不依赖，共享 Foundation 产物 |
| 阶段 3 | Final Plan | 1 | 必须等 Plan 2 和 Plan 3 都完成 |

**关键并发点**：阶段 2 的 Plan 2 与 Plan 3 可在两个独立 worktree 中并发执行。

## 4. 整体约束（所有 Plan 共享）

### 4.1 Must Have

- 1:1 函数映射，每个 Rust fn 标注 `// c: xxx.c:LINE`
- 所有 on-flash struct `#[repr(C)]` + `size_of` 编译期验证
- CRC32 与 C 版字节级一致
- 所有条件编译 → Cargo features（禁止运行时 if）
- UT 每个断言有真实业务逻辑验证（禁止虚假断言，如 `assert!(true)`）
- **C 等价性测试**：原 C 项目 `tests/` 的测试数据迁移为 Rust integration test，对照 C 版输出验证

### 4.2 Must NOT Have（Guardrails）

- ❌ on-flash 二进制格式变更
- ❌ `unsafe { transmute }` 在非 FFI 边界
- ❌ `unwrap()` / `expect()` 对 Result
- ❌ `impl Deref for Child` 模拟继承
- ❌ `unimplemented!()` / `todo!()` 占位
- ❌ `_` 前缀变量名
- ❌ 合并多个 struct 到同一文件
- ❌ 简化/硬编码/伪造默认值
- ❌ RT-Thread 相关依赖
- ❌ FAL 具体实现依赖（第一阶段）

### 4.3 Definition of Done（整体）

- [ ] `cargo build` 成功，零 warning
- [ ] `cargo test` 所有 UT + C 等价性测试通过
- [ ] `cargo test --test bdd` 6 个 feature 全部通过
- [ ] `cargo clippy -- -D warnings` 零 warning
- [ ] 所有 on-flash struct size_of 验证通过
- [ ] BDD 场景全部覆盖（与 6 个 feature 文件 1:1 对应）
- [ ] 零 `unsafe` 在非 FFI 边界使用
- [ ] 每个 C 公共函数有对应 Rust fn（1:1 映射）
- [ ] 函数级覆盖率矩阵审计通过（每个公共函数 ≥3 case）

---