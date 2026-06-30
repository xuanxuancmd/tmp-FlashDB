# Final Plan: 集成 + BDD + 验证

> **总目标**：见 `00-start.md` — FlashDB C→Rust 1:1 翻译迁移
>
> **本 Plan 目标**：集成验证 Plan 1-3 产物，实现 BDD Step Definition（FT 层），执行 E2E 测试，完成代码检视。**不写功能代码** — 所有功能代码已在子 Plan 中完成。

---

## 1. IN / OUT Scope

### IN Scope
- 集成编译：Plan 1 + Plan 2 + Plan 3 产物合并后无冲突编译
- BDD cucumber-rust 框架搭建（`tests/bdd/`）
- 6 个 `.feature` 文件的 Step Definition 实现（共 66 个 scenario）
- E2E 测试规格设计 + 执行（声明式 YAML）
- 4 项最终验证：Plan 合规审计、代码质量、真实 QA、范围保真

### OUT Scope
- ❌ 任何功能代码修改（如发现 bug，提 issue 给对应子 Plan 修复后重新进入 Final）
- ❌ 修改 `.feature` 文件内容（已审核锁定）
- ❌ 新增 BDD scenario（如确需，回退到需求分析流程重新走 §2 审核）

---

## 2. 四项责任（严格顺序）

> 按 `requirement-analysis-helper` §3.3 Final Plan 准入要求，依次完成 4 项责任。每项责任完成并通过后才能进入下一项。

### 任务清单

- [ ] **责任 1**：集成编译（合并 Plan 2/3 产物 + `cargo build --all-features` + `cargo test --lib` + `cargo test --test c-port`）
- [ ] **T19**：BDD cucumber-rust 集成框架搭建（World + 6 feature 注册）
- [ ] **T20**：BDD 场景实现 — KVDB init + CRUD（2 feature, 20 scenarios）
- [ ] **T21**：BDD 场景实现 — KVDB iteration + GC（1 feature, 8 scenarios）
- [ ] **T22**：BDD 场景实现 — TSDB init（1 feature, 11 scenarios）
- [ ] **T23**：BDD 场景实现 — TSDB append（1 feature, 13 scenarios）
- [ ] **T24**：BDD 场景实现 — TSDB query + management（1 feature, 14 scenarios）
- [ ] **T0-E2E**：E2E 测试规格设计 + 执行（声明式 YAML）
- [ ] **F1-F4**：代码检视（4 项并行审计全部 APPROVE）

---

### 责任 1: 集成编译（前置）

**准入条件**：Plan 1、Plan 2、Plan 3 全部出口条件满足。

**What to do**:
- 合并 Plan 2 和 Plan 3 的 worktree（如使用并发 worktree 模式）到主分支
- 解决 `src/lib.rs` 的 mod 声明合并冲突（Plan 2 添加 `mod kvdb;`，Plan 3 添加 `mod tsdb;`）
- 运行 `cargo build` 确保零编译错误
- 运行 `cargo build --all-features` 确保所有 feature 组合编译通过
- 运行 `cargo test --lib` 确保 Plan 1-3 的所有 UT 在合并后仍全部通过
- 运行 `cargo test --test c-port` 确保所有 C 等价性 integration test 通过（`tests/c-port/foundation_equiv.rs` + `kvdb_equiv.rs` + `tsdb_equiv.rs`）

**Acceptance Criteria**:
- [ ] `cargo build` 零错误，零 warning
- [ ] `cargo build --all-features` 零错误
- [ ] `cargo test --lib` 全部 UT 通过（无回归）
- [ ] `cargo test --test c-port` 全部 C 等价性测试通过

---

### 责任 2: FT 实现 + 执行（BDD Step Definition）

**准入条件**：责任 1 通过。

#### T19. BDD cucumber-rust 集成框架搭建

**What to do**:
- 添加 BDD 测试 infrastructure：
  - `Cargo.toml` 添加 dev-dependencies: `cucumber = "0.21"` + `tokio`
  - `features/` 目录复制现有 6 个 `.feature` 文件（中文 Gherkin，来自 `.opencode/harness/features/`）
  - `tests/bdd/` 目录创建 step definition 文件
  - 创建 `tests/bdd/mod.rs` — cucumber World 定义（包含 FlashDB 实例 + MockFlash）
  - 注册所有 6 个 feature 文件的 scenario
- World struct 实现：
  - `struct FlashWorld { flash: MockFlash, kvdb: FdbKvdb, tsdb: FdbTsdb, ... }`
  - 每个 feature 文件对应的 step 模块

**Must NOT do**:
- 不要修改现有 `.feature` 文件内容
- 不要跳过任何 scenario

**Recommended Agent Profile**:
- Category: `quick`

**Parallelization**:
- Can Run In Parallel: NO（Final Plan 内部串行）
- Blocks: T20-T24
- Blocked By: 责任 1 完成

**References**:
- `.opencode/harness/features/` — 6 个 Gherkin feature 文件（已审核）

**Acceptance Criteria**:
- [ ] `cargo test --test bdd` 编译通过（step 可暂时 unimplemented）
- [ ] `features/` 目录有 6 个 `.feature` 文件

**QA Scenarios**:
```
Scenario: BDD 框架编译通过
  Tool: Bash
  Steps:
    1. cargo build --tests
    2. 检查 features/ 目录有 6 个 .feature 文件
  Expected Result: 编译成功，feature 文件完整
```

**Commit**: NO（与 T24 末合并提交）

---

#### T20. BDD 场景实现: KVDB init + CRUD (2 feature files)

**What to do**:
- 实现 `features/kvdb-init.feature` 全部 10 个 scenario
- 实现 `features/kvdb-crud.feature` 全部 10 个 scenario
- 每个 scenario 的 Given/When/Then step 绑定真实 FlashDB API 调用（来自 Plan 2 公共 API）
- Flash 后端使用 `MockFlash`
- 断言必须有真实验证（禁止虚假断言）

**Must NOT do**:
- 不要修改 feature 文件内容
- 不要使用 fake assertions（如 `assert!(true)`）

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES（与 T21 并发，两者互不依赖）
- Blocks: 责任 3
- Blocked By: T19, Plan 2 (T13)

**References**:
- `.opencode/harness/features/kvdb-init.feature` — KVDB init BDD（10 scenarios）
- `.opencode/harness/features/kvdb-crud.feature` — KVDB CRUD BDD（10 scenarios）

**Acceptance Criteria**:
- [ ] `cargo test --test bdd -- features/kvdb-init.feature` 全部 scenario 通过
- [ ] `cargo test --test bdd -- features/kvdb-crud.feature` 全部 scenario 通过

**QA Scenarios**:
```
Scenario: KVDB init feature 全部通过
  Tool: Bash
  Steps:
    1. cargo test --test bdd -- features/kvdb-init.feature
  Expected Result: 10 scenarios 全部 PASS
```

**Commit**: NO（与 T24 末合并提交）

---

#### T21. BDD 场景实现: KVDB iteration + GC (1 feature file)

**What to do**:
- 实现 `features/kvdb-iteration-gc.feature` 全部 8 个 scenario
- 每个 scenario 绑定真实 API 调用
- 断言必须有真实验证

**Must NOT do**:
- 不要修改 feature 文件
- 不要虚假断言

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES（与 T20 并发）
- Blocks: 责任 3
- Blocked By: T19, Plan 2 (T13)

**References**:
- `.opencode/harness/features/kvdb-iteration-gc.feature` — 8 scenarios

**Acceptance Criteria**:
- [ ] `cargo test --test bdd -- features/kvdb-iteration-gc.feature` 全部通过

**QA Scenarios**:
```
Scenario: KVDB iteration + GC feature 全部通过
  Tool: Bash
  Steps:
    1. cargo test --test bdd -- features/kvdb-iteration-gc.feature
  Expected Result: 8 scenarios 全部 PASS
```

**Commit**: NO（与 T24 末合并提交）

---

#### T22. BDD 场景实现: TSDB init (1 feature file)

**What to do**:
- 实现 `features/tsdb-init.feature` 全部 11 个 scenario
- 绑定真实 API 调用（来自 Plan 3 公共 API）
- 断言必须有真实验证

**Must NOT do**:
- 不要修改 feature 文件
- 不要虚假断言

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES（与 T23, T24 并发）
- Blocks: 责任 3
- Blocked By: T19, Plan 3 (T16, T18)

**References**:
- `.opencode/harness/features/tsdb-init.feature` — 11 scenarios

**Acceptance Criteria**:
- [ ] `cargo test --test bdd -- features/tsdb-init.feature` 全部通过

**QA Scenarios**:
```
Scenario: TSDB init feature 全部通过
  Tool: Bash
  Steps:
    1. cargo test --test bdd -- features/tsdb-init.feature
  Expected Result: 11 scenarios 全部 PASS
```

**Commit**: NO（与 T24 末合并提交）

---

#### T23. BDD 场景实现: TSDB append (1 feature file)

**What to do**:
- 实现 `features/tsdb-append.feature` 全部 13 个 scenario
- 绑定真实 API 调用
- 断言必须有真实验证

**Must NOT do**:
- 不要修改 feature 文件
- 不要虚假断言

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES（与 T22, T24 并发）
- Blocks: 责任 3
- Blocked By: T19, Plan 3 (T16, T18)

**References**:
- `.opencode/harness/features/tsdb-append.feature` — 13 scenarios

**Acceptance Criteria**:
- [ ] `cargo test --test bdd -- features/tsdb-append.feature` 全部通过

**QA Scenarios**:
```
Scenario: TSDB append feature 全部通过
  Tool: Bash
  Steps:
    1. cargo test --test bdd -- features/tsdb-append.feature
  Expected Result: 13 scenarios 全部 PASS
```

**Commit**: NO（与 T24 末合并提交）

---

#### T24. BDD 场景实现: TSDB query + management (1 feature file)

**What to do**:
- 实现 `features/tsdb-query-management.feature` 全部 14 个 scenario
- 绑定真实 API 调用
- 断言必须有真实验证

**Must NOT do**:
- 不要修改 feature 文件
- 不要虚假断言

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Parallelization**:
- Can Run In Parallel: YES（与 T22, T23 并发）
- Blocks: 责任 3
- Blocked By: T19, Plan 3 (T17, T18)

**References**:
- `.opencode/harness/features/tsdb-query-management.feature` — 14 scenarios

**Acceptance Criteria**:
- [ ] `cargo test --test bdd -- features/tsdb-query-management.feature` 全部通过

**QA Scenarios**:
```
Scenario: TSDB query + management feature 全部通过
  Tool: Bash
  Steps:
    1. cargo test --test bdd -- features/tsdb-query-management.feature
  Expected Result: 14 scenarios 全部 PASS
```

**Commit**: YES
- Message: `test(bdd): 所有 BDD 场景完成（6 feature files, 66 scenarios）+ cucumber-rust 集成`

**BDD 总览（责任 2 完成判定）**:

| Feature 文件 | Scenario 数 | 任务 | 完成标志 |
|-------------|------------|------|---------|
| kvdb-init.feature | 10 | T20 | cargo test 全 PASS |
| kvdb-crud.feature | 10 | T20 | cargo test 全 PASS |
| kvdb-iteration-gc.feature | 8 | T21 | cargo test 全 PASS |
| tsdb-init.feature | 11 | T22 | cargo test 全 PASS |
| tsdb-append.feature | 13 | T23 | cargo test 全 PASS |
| tsdb-query-management.feature | 14 | T24 | cargo test 全 PASS |
| **合计** | **66** | | |

---

### 责任 3: E2E 执行

**准入条件**：责任 2 通过（所有 BDD scenario PASS）。

#### T0-E2E. E2E 测试规格设计 + 执行

> ⚠️ **Gap 提示**：原 Plan 未包含 E2E 规格。按 `requirement-analysis-helper` §2.2，本责任执行前需先完成声明式 E2E YAML 设计。

**What to do**:

1. **设计阶段**（执行前）：
   - 生成声明式 E2E YAML 覆盖以下场景（BDD 无法覆盖的）：
     - 进程管理：crate 编译为静态库被外部链接（可选，超出第一阶段范围标记为 deferred）
     - 环境配置：不同 Cargo feature 组合（`kvdb`, `tsdb`, `gran_8/32/64/128/256`, `file_mode`, `kv_cache`, `timestamp_64bit`）的编译验证
     - 多组件编排：KVDB 与 TSDB 共存于同一 FlashDevice 实例的隔离性验证
   - 人工审核 E2E YAML

2. **执行阶段**：
   - 执行 E2E YAML 中所有 scenario
   - 每个 scenario 的 `when` 和 `then` 必须非空完整
   - 验证 E2E 覆盖 BDD 的所有业务路径

**Must NOT do**:
- 不要跳过 E2E 设计直接执行（先设计后执行）
- 不要简化 E2E scenario

**Recommended Agent Profile**:
- Category: `unspecified-high`

**Acceptance Criteria**:
- [ ] E2E YAML 文件存在且经人工审核
- [ ] 每个 scenario 的 `when` 和 `then` 非空完整
- [ ] 所有 E2E scenario 执行通过
- [ ] E2E 覆盖 BDD 的所有业务路径（人工确认）

---

### 责任 4: 代码检视

**准入条件**：责任 3 通过。

> 4 项并行审计。**ALL must APPROVE**。汇总结果给用户，获得明确 "okay" 后才能完成 Final Plan。

#### F1. Plan Compliance Audit

**What to do**:
- Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, cargo test output). For each "Must NOT Have": search codebase for forbidden patterns. Compare deliverables against plan.
- 特别检查：
  - 所有 on-flash struct 有 `#[repr(C)]` + size_of 验证
  - 所有 fn 有 `// c: xxx.c:LINE` 注释
  - FAL 和 RT-Thread 集成层未实现
  - BDD feature 文件与 6 个原始 `.feature` 文件内容一致

**Output**: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

---

#### F2. Code Quality Review

**What to do**:
- Run `cargo clippy -- -D warnings` + `cargo test`
- Check:
  - `unwrap()`/`expect()` prohibited
  - `unsafe` outside FFI
  - `_` prefix variables
  - `unimplemented!()`/`todo!()` placeholders
  - fake assertions
  - AI slop

**Output**: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

---

#### F3. Real Manual QA

**What to do**:
- Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence
- Run `cargo test` and `cargo test --test bdd`
- Verify MockFlash NOR flash semantics
- Verify CRC32 matches C
- Verify on-flash struct sizes match C

**Output**: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

---

#### F4. Scope Fidelity Check

**What to do**:
- For each task: read "What to do", read actual diff. Verify 1:1 — everything in spec built (no missing), nothing beyond spec built (no creep). Check "Must NOT do" compliance.
- 特别检查：
  - C 源码每个公共函数有对应 Rust fn（不遗漏）
  - 未擅自添加新功能
  - BDD step 与 feature 文件 scenario 1:1 对应
  - 测试数据对齐
- **函数级覆盖率审计**：生成覆盖率矩阵，行 = 所有公共函数（KVDB + TSDB + Foundation），列 = {normal path, error path, boundary case, C 等价性测试}。审计每个函数至少 3 个 case 覆盖，标记缺失项。缺失项回退到对应子 Plan 补齐。

**Output**: `Tasks [N/N compliant] | Coverage [N/N functions ≥3 cases] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## 3. 测试分层执行总结（§3.4 多 Plan 模式）

| 层级 | 执行时机 | 由谁实现 | 状态 |
|------|---------|---------|------|
| UT | 子 Plan 编码后立即 | Plan 1/2/3 | ⏳ 待执行 |
| FT (BDD Step Def) | 责任 2 | 本 Final Plan (T19-T24) | ⏳ 待执行 |
| E2E | 责任 3 (FT 通过后) | 本 Final Plan (T0-E2E) | ⏳ 待执行 |
| 代码检视 | 责任 4 (E2E 通过后) | 本 Final Plan (F1-F4) | ⏳ 待执行 |

---

## 4. 出口条件（Final Plan 完成判定）

- [ ] 责任 1: `cargo build --all-features` 零错误，`cargo test --lib` 全部 UT 通过（无回归）
- [ ] 责任 2: `cargo test --test bdd` 6 个 feature 文件全部 66 个 scenario PASS
- [ ] 责任 3: E2E YAML 设计完成 + 执行通过 + 覆盖 BDD 业务路径
- [ ] 责任 4: F1/F2/F3/F4 全部 APPROVE，用户明确 "okay"
- [ ] 整体 DoD（`00-start.md` §5.3）全部满足
- [ ] 2 次 Commit 完成（T19 后 + T24 后）

---

## 5. 失败处理

### 5.1 集成编译失败（责任 1）

- 若发现 Plan 2/3 产物冲突 → 回退到对应子 Plan 修复，重新进入 Final Plan 责任 1
- 若发现 Foundation API 不满足 Plan 2/3 需求 → 回退到 Plan 1 补充，重新进入 Final Plan 责任 1

### 5.2 BDD step 失败（责任 2）

- 若 step 绑定的 API 不存在 → 检查 Plan 2/3 下游契约（§2.2），若 API 未实现则回退到对应 Plan 修复
- 若 step 断言失败 → 检查对应 Plan 的 UT 是否漏掉该 case，回退补充 UT + 修复功能代码
- 若 feature 文件 scenario 与实现不符 → **禁止修改 feature 文件**，回退到需求分析流程重新审核

### 5.3 E2E 失败（责任 3）

- 若 E2E scenario 失败 → 分析根因，回退到对应 Plan 修复

### 5.4 代码检视失败（责任 4）

- F1 REJECT → 补齐缺失项，重新审计
- F2 REJECT → 修复质量问题（禁止 `unwrap`、`unsafe` 等），重新审计
- F3 REJECT → 补充 QA 证据，重新执行失败 scenario
- F4 REJECT → 移除超范围代码或补齐缺失功能，重新审计

3 次连续退化升级人工处理。

---

## 6. 提交（Commit）策略

| Commit 时机 | Message |
|------------|---------|
| T19 后 | `test(bdd): cucumber-rust 集成框架 + World 定义 + 6 feature 文件注册` |
| T24 后 | `test(bdd): 6 feature files 全场景覆盖 (66 scenarios) + E2E + 代码检视通过` |
