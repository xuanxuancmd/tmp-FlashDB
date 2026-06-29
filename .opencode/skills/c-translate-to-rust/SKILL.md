---
name: c-translate-to-rust
description: C 到 Rust 1:1 代码翻译实战指南。提供 C 语法到 Rust 的映射速查表、Rust 禁令表、翻译易错表，以及嵌入式 C 特有的复杂场景（flash 存储、条件编译、指针运算、RTOS 集成）详细参考方案。当用户提到 C 转 Rust、C 代码迁移 Rust、嵌入式 C 重写 Rust、flash 数据库 Rust 移植等场景时必须使用此 skill。
---

# C→Rust 翻译实战指南

## 职责

本 Skill 为 AI 编码 Agent 提供 C→Rust 代码翻译的规则与决策框架。

## 使用前必读

+ 不在本文描述的 C→Rust 规则通常较为简单，模型可以自主决策。
+ 从上下文或 AGENTS.md 中查询 C 源码的位置，若未找到应通过 `question()` 向用户询问。
+ 翻译默认遵从 1:1 原则，确保功能不遗漏，但同时也需遵从 Rust 语言特点（所有权、借用、生命周期）。
+ 嵌入式 C 代码迁移需特别关注：on-flash 二进制格式兼容性、no_std/no_alloc 约束、条件编译矩阵。

## 工作流程

### Step 1：翻译基本原则

+ **1:1 翻译约束**：代码翻译保证结构体、函数名 1:1 对齐（采用 Rust snake_case 命名风格）；语言差异过大无法 1:1 映射时，需重点分析迁移方案并提交用户审阅
    - **函数归属规则**：识别 C 的隐式 receiver（第一个 `某struct_t` 参数），按它拆分 impl 块：
      - 第一个参数是主 struct 指针 → 主 struct 的 `impl` 方法
      - 操作内部辅助 struct（如 on-flash header）→ 辅助 struct 的私有 `impl`（`pub(crate)`）
      - 无 struct 指针参数 → 模块级自由 `fn`
      - **禁止**将所有函数强行塞入主 struct 的 `impl`
+ **源码单元映射**：C 的 `.h`（声明）+ `.c`（实现）= 一个语义单元 = 一个 `.rs` 模块
    - 类型定义（来自 `.h`）放模块顶部，`impl` 块（来自 `.c`）放下方，不拆分"声明文件"和"实现文件"
    - Rust 不保留 C 的 `inc/` vs `src/` 目录区分，所有 `.rs` 统一放在 `src/` 下
    - 公共 API header（项目的 "main.h"）的声明职责由 `lib.rs` 的 `pub use` 重导出替代
    - 编译时配置头文件（如 `*_cfg.h`、`*_config.h`）→ `Cargo.toml` features + `build.rs`，不生成 `.rs` 文件
    - 跨模块共享的最小类型集（被 2+ 模块引用的 enum/struct）可提取到独立模块，但**不是** 1:1 搬运整个 header——只放真正共享的最小集合
    - 示例代码、测试：`samples/` → `examples/`（可选），`tests/` → 对应模块的 `#[cfg(test)] mod tests` 或 `tests/` 目录
+ **on-flash 格式兼容**：所有用于 flash 序列化的 C struct 必须用 `#[repr(C)]` 翻译，并通过 `assert_eq!(core::mem::size_of::<Struct>(), expected)` 验证布局一致

### Step 2：加载翻译基线（每次翻译前必读）

"核心翻译规则"章节的四张表（§1 强制翻译表、§2 建议翻译表、§3 禁令表、§4 易错表）是翻译的规则手册，任何涉及 C→Rust 转换的编码前必须内化。

**注意：尤其是上述表中第一列，起到目录导航作用**

### Step 3：翻译中按需加载 references

+ 简单场景，通过翻译表直接查看，对应章节 §1-A
+ 复杂场景，对应章节 §1-B，翻译表中的场景行会标注"详见 `references/xxx.md`"，如果 C 场景匹配到复杂场景后，就需要加载对应 reference 文件，读取完整方案后再动手翻译

### Step 4：翻译后必查

+ **易错检查**：对照 §4 翻译易错表
+ **禁令验证**：对照 §3 Rust 禁令表，确保无违禁模式
+ **与源码对照**：所有修改必须与 C 源码对照，确保 1:1 映射
+ **on-flash 验证**：涉及 flash 序列化的 struct，必须验证 `size_of` 与 C 版本一致

---

## 核心翻译规则

### §1 强制翻译表 [强制应用]

#### A. 简单规则 — 直接应用

| C 特性 | Rust 替代 | 迁移规则 | 代码举例 |
|--------|----------|---------|---------|
| `#define CONST 42` | `const CONST: u32 = 42;` | 始终添加类型标注；文件作用域用 `pub(crate)`，导出用 `pub` | `const SECTOR_MAGIC_WORD: u32 = 0x30424446;` |
| `typedef enum { A, B } e_t;` | `#[repr(u8)] enum E { A, B }` | 跨 FFI 或持久化用 `#[repr(C)]`；仅内部用 `#[repr(u8)]` | `#[repr(C)] pub enum FdbErr { NoErr = 0, EraseErr, ReadErr, WriteErr }` |
| `typedef struct X *X_t;` | 直接用 `&mut X` / `&X` | 指针 typedef 消除，用引用替代；需生命周期时标注 `<'a>` | `fn init(db: &mut Kvdb) -> Result<(), FdbErr>` |
| `memcpy(dst, src, n)` | `dst.copy_from_slice(&src[..n])` | 长度必须匹配；部分拷贝用 `dst[..n].copy_from_slice(&src[..n])` | `sec_hdr.copy_from_slice(&buf[..size]);` |
| `memset(buf, 0xFF, n)` | `buf.fill(0xFF)` | 0 初始化用 `[0u8; N]` | `buf.fill(FDB_BYTE_ERASED);` |
| `memset(&s, 0, sizeof s)` | `s = Default::default()` 或 `[0u8; N]` | 优先 `#[derive(Default)]` | `let sec_hdr: SectorHdr = Default::default();` |
| `strlen(s)` | `s.len()` (字节长度) | 注意 C `strlen` 是字节计数不含 `\0`，Rust `len()` 也是 | — |
| `strcmp(a, b)` / `strncmp` | `a == b` / `a.starts_with(b)` | 字节比较用 `.as_bytes() == b.as_bytes()` | — |
| `strncpy(dst, src, n)` | `dst[..n].copy_from_slice(&src[..n])` | 确保长度安全 | — |
| `while(1)` / `for(;;)` | `loop {}` | `loop` 可通过 `break expr` 返回值 | — |
| `for(i=0; i<n; i++)` | `for i in 0..n` | 半开区间，与 C `for(i=0;i<n;i++)` 语义一致 | — |
| `switch/case`(有 break) | `match` | 无隐式 fallthrough；合并分支用 `\|` | `match cmd { 0x00 => ..., 0x01 => ..., _ => panic!() }` |
| `bool`/`true`/`false` | `bool`/`true`/`false` | C99 `<stdbool.h>` 直接映射 | — |
| `uint8_t buf[32]` | `let mut buf = [0u8; 32];` | 固定大小数组；宏大小用 `const N: usize` | — |
| `sizeof(arr)/sizeof(arr[0])` | `arr.len()` | 切片和数组都有 `.len()` | — |
| `#include` 头文件守卫 | Cargo 模块系统 | 无需 `#ifndef` 守卫；`mod` 声明即可 | — |
| `extern "C" { ... }` | `extern "C" { ... }` + `#[no_mangle]` | FFI 边界保持 ABI 兼容 | — |
| `assert(x)` / 自定义 ASSERT | `assert!(x)` / `debug_assert!(x)` | 嵌入式 `while(1)` assert → `panic!()` | — |
| `snprintf(buf, sz, fmt, ...)` | `write!(&mut buf, fmt, args)` | no_std 用 `heapless::String`；std 用 `format!` | `write!(buf, "{}.fdb.{}", name, index)?;` |
| 指定初始化 `.field = val` | 结构体字面量 `S { field: val }` | 部分初始化用 `..Default::default()` | `SectorHdr { addr: 0, ..Default::default() }` |
| 三元 `cond ? a : b` | `if cond { a } else { b }` | — | — |
| `__FILE__` / `__LINE__` / `__func__` | `file!()` / `line!()` / `module_path!()` | 函数名无稳定宏，用 `#[track_caller]` + `Location::caller()` | — |
| `NULL` / 空指针 | `Option<T>` / `None` | 利用空指针优化(NPO)：`Option<&T>` 与 `*const T` 大小相同 | `fn find(name: &str) -> Option<&Kv>` |
| `return -1`(错误) / `return size`(成功) | `Result<usize, Error>` | 负数=错误 → `Err`；非负=成功 → `Ok` | `fn read(&self, buf: &mut [u8]) -> Result<usize, FlashErr>` |
| `#error "msg"` | `compile_error!("msg")` | 配合 `#[cfg]` 条件触发 | — |

#### B. 复杂规则 — 先查 references 再翻译

| C 特性 | C 示例 | Rust 映射规则 | 参考文档 |
|--------|--------|--------------|---------|
| 指针强转类型双关 `(uint32_t*)buf` | `(uint32_t *)&sec_hdr` 传给 flash 读写 | `zerocopy::AsBytes`/`FromBytes` 或 `bytemuck::cast`；**禁止 `transmute`** | 详见 [type-punning.md](references/type-punning.md) |
| offsetof via null 解引用 `&((struct X*)0)->field` | `#define KV_MAGIC_OFFSET ((unsigned long)(&((struct kv_hdr_data *)0)->magic))` | `core::mem::offset_of!`（Rust 1.77+）或 `memoffset::offset_of!` crate | 详见 [offsetof.md](references/offsetof.md) |
| `#ifdef` 改变 struct 布局 | `struct fdb_db` 按 `FDB_USING_FAL_MODE` 添加字段 | const generic + `#[cfg]` + `#[repr(C)]`；**每种配置独立验证 `size_of`** | 详见 [conditional-struct-layout.md](references/conditional-struct-layout.md) |
| C 继承（struct 嵌入） `struct child { struct parent; }` | `struct fdb_kvdb { struct fdb_db parent; ... }` | 组合 + `AsRef<Parent>` trait；**禁止 `Deref` 模拟继承** | 详见 [c-inheritance.md](references/c-inheritance.md) |
| `goto __exit`(清理) / `goto __retry`(重试) / `goto __reload`(前向跳转) | `goto __exit; ... __exit: cleanup();` | `?` + `Drop`(RAII) / `loop { break }` / 状态机重构 | 详见 [goto-patterns.md](references/goto-patterns.md) |
| `void*` 回调参数 `bool (*cb)(T*, void* arg1, void* arg2)` | `kv_iterator(db, kv, (void*)key, &find_ok, find_kv_cb)` | 泛型闭包 `FnMut(&mut T, &A) -> bool` 或 trait 对象 | 详见 [void-callbacks.md](references/void-callbacks.md) |
| `#ifdef` 矩阵（如 `FDB_WRITE_GRAN` × 6 种值） | `#if (FDB_WRITE_GRAN == 64)` 改变 padding 和状态表大小 | Cargo features + const generic `struct FlashDb<const GRAN: u32>`；**on-flash 格式不可变** | 详见 [feature-flags.md](references/feature-flags.md) |
| `do { ... } while(0)` 宏含 `return` | `_FDB_WRITE_STATUS(...)` 宏内 `return result;` | 重构为函数返回 `Result` + `?` 操作符 | 详见 [macro-to-fn.md](references/macro-to-fn.md) |
| C 函数指针 vtable `struct ops { int (*read)(...); int (*write)(...); }` | FAL `fal_flash_dev.ops` | `trait FlashDevice { fn read/erase/write }` + `embedded-storage` 适配 | 详见 [fal-to-trait.md](references/fal-to-trait.md) |
| `void*` → 函数指针强转 `(void(*)(fdb_db_t))arg` | `fdb_kvdb_control` 中 SET_LOCK 命令 | **重新设计**：用 enum 或 builder 替代命令模式；`#pragma GCC diagnostic` 必须删除 | 详见 [fn-ptr-cast.md](references/fn-ptr-cast.md) |
| 内存映射 flash 读取 `*(uint8_t*)addr` | STM32 HAL `*buf = *(uint8_t *) addr;` | `unsafe { core::ptr::read_volatile(addr) }`；**C 缺少 volatile 是 bug** | 详见 [mmio.md](references/mmio.md) |
| `static` 可变全局变量 `static uint8_t init_ok = 0;` | FAL `static uint8_t init_ok = 0;` | `AtomicBool`/`AtomicI32` 或 `OnceLock<T>`；**禁止 `static mut`** | 详见 [global-state.md](references/global-state.md) |
| union 类型 `union { A*; B*; }` | `struct fdb_db` 的 `union storage` | `enum Storage { Fal(Partition), File(PathBuf) }`；FFI 边界可用 `unsafe union` | — |


### §2 建议翻译表 [提交用户审阅后采用]

以下映射涉及架构决策，**不可直接应用**，必须向用户说明并获审阅。

**注意：如果涉及如下"C 特性"，请使用"提问说明"向用户提问。如果接受建议方案，则读取详细参考。**

| C 特性 | Rust 建议方案 | 提问说明 | 详细参考 |
|--------|-------------|---------|---------|
| RT-Thread / RTOS 集成（`fal_rtt.c` 等设备模型） | 丢弃 RTOS 适配层，用 `embedded-storage` trait 替代 | "RTOS 设备模型（如 RT-Thread `rt_device`）与 Rust 嵌入式生态不兼容。是否接受丢弃 RTOS 集成层，改用 `embedded-storage` 标准 trait？" | — |
| FAL flash 设备 vtable | 自建 `trait FlashDevice` + `embedded-storage::NorFlash` 适配层 | "FAL 的函数指针 vtable 应映射为 Rust trait。是否接受引入 `embedded-storage` crate 作为 flash 抽象层？" | [fal-to-trait.md](references/fal-to-trait.md) |
| `fdb_kv_get()` 返回 static buffer 指针 | 改为返回 `Option<String>` 或 `Option<Cow<str>>` | "C API 返回内部 static buffer 指针，非线程安全。Rust 版改为返回拥有所有权的 `String`，API 签名变化。是否接受此破坏性变更？" | — |
| `FDB_WRITE_GRAN` 6 种值（1/8/32/64/128/256） | const generic `struct FlashDb<const GRAN: u32>` 编译时单态化 | "C 用 `#if (FDB_WRITE_GRAN == 64)` 选择布局，Rust 建议用 const generic 编译时单态化，每种 GRAN 生成独立类型。是否接受？" | [feature-flags.md](references/feature-flags.md) |
| `fdb_kvdb_control(db, cmd, arg)` 命令模式 | Builder 模式 + `Default` trait + 类型安全的 setter | "C 的 `control(db, SET_SEC_SIZE, &val)` 用 `void*` 传参，类型不安全。Rust 建议改为 builder 模式。是否接受 API 重构？" | [fn-ptr-cast.md](references/fn-ptr-cast.md) |
| C 宏函数 `#define db_name(db) (((fdb_db_t)db)->name)` | Rust 方法 `fn name(&self) -> &str` | "C 用宏模拟继承访问器，Rust 直接用方法。是否接受消除所有访问器宏？" | [c-inheritance.md](references/c-inheritance.md) |


### §3 禁令表 [必读·零容忍]

**如下语言级禁令，除非用户许可否则禁止使用：**

| 禁止事项 | 原因 |
|---------|------|
| 对 Result 变量使用 `unwrap()` 或 `expect()` | 可能导致进程 panic |
| C 独立 struct 对应多个 struct 合并同一 `.rs` 文件 | 破坏 1:1 映射，代码不可追踪 |
| `std::sync::Mutex` / `std::sync::RwLock` 在 `no_std` 环境 | 应使用 `critical_section::Mutex` 或 `embassy_sync::Mutex` 适配嵌入式 |
| `block_in_place` / `block_on` / `blocking_lock` / `blocking_read` / `blocking_write` | 可能导致线程耗尽从而死锁 |
| `Mutex<primitive>`（如 `Mutex<u64>`、`Mutex<bool>`） | 应使用 Atomic* 代替，性能更安全 |
| `log!` 宏内部使用 `await` | log 宏不支持非 Send 代码 |
| `try_lock`（非阻塞锁获取） | 除 `std::fmt::Debug` 实现外，其余地方禁止使用 |
| `static mut` | Rust 2024 已弃用；应使用 `Atomic*` / `Mutex<T>` / `OnceLock<T>` |
| `unsafe { core::mem::transmute }` 在非 FFI 边界 | 应使用 `zerocopy`/`bytemuck` 安全转换；transmute 是 code smell |
| `core::mem::zeroed()` 用于非 Copy 类型 | 对引用/bool/enum 是 UB；union 用具名字段初始化 |
| `impl Deref for Child` 模拟 C 继承 | 反模式；应使用组合 + `AsRef<Parent>` |
| `Result<T, ()>` 或 `Result<T, String>` 作为公共 API 返回类型 | 违反 API Guidelines C-GOOD-ERR；必须定义实现 `Error` trait 的错误枚举 |
| `catch_unwind` 跨 FFI 边界捕获 C 异常 | Nomicon 明确标注为 UB |
| 修改 on-flash 二进制格式（struct 布局、magic word、CRC 多项式） | FlashDB 的价值在于 on-flash 格式兼容性；格式变化导致数据丢失 |

**翻译过程禁令：**

+ ❌ C 可变参宏 `#define F(x, ...)` → 必须翻译为 `macro_rules!`，**禁止**翻译为 `unsafe` 函数
+ ❌ `#ifdef` 条件编译 → 必须翻译为 `#[cfg(feature)]` 或 const generic，**禁止**翻译为运行时 `if`
+ ❌ on-flash struct 添加 Rust 专用字段（如 `PhantomData`、trait object），**禁止**改变 `#[repr(C)]` 布局
+ ❌ `goto` → **禁止**直接翻译为 `unsafe { goto }`（Rust 无 goto），必须重构为 `?`/`loop`/状态机

**禁令例外：**
+ `std::fmt::Debug` 实现中允许使用 `try_lock`，避免使用 async/await 的编译报错
+ FFI 边界（`extern "C"` 函数内部）允许 `unsafe { transmute }` 转换函数指针，但必须有详细安全注释
+ `unsafe union` 仅允许在 FFI 边界使用，内部代码必须用 `enum`


### §4 易错表 [必读]

#### A. C → Rust 映射易错表

| C 特性 | Rust 注意事项 | 关键提醒 |
|--------|-------------|---------|
| `#[repr(C)]` 但忘记 `#[repr(C, packed)]` | padding 不匹配 C 版本 | flash 序列化 struct 必须验证 `size_of` 与 C 一致；条件 padding 需按 `FDB_WRITE_GRAN` 分别验证 |
| `core::mem::offset_of!` 需要 Rust 1.77+ | 低版本编译失败 | 用 `memoffset` crate 作为替代；升级工具链到 1.77+ |
| `#[cfg(feature)]` 改变 struct 大小 | 编译错误或布局不一致 | 用 const generic 或拆分为独立类型；**不可**在同一 struct 上用 `#[cfg]` 添加/删除字段 |
| `const fn` 不能用 `if`（旧版 Rust） | 编译失败 | Rust 1.46+ 支持 `const fn` 中的 `if`；用 `#[cfg]` 分支或 `match` |
| `Drop` trait 中做 flash I/O | panic 不可恢复 | `Drop` 只做内存清理；flash 同步在显式 `flush()` 方法中完成 |
| `&mut self` 借用与回调闭包冲突 | 编译错误 | 回调改为 `FnMut` 或将状态分离到独立字段；GC 中 `alloc_kv` 调 `move_kv` 调 `alloc_kv` 需重构所有权 |
| `extern "C"` 函数返回 `Result<T, E>` | ABI 不兼容 | FFI 函数返回原始整数错误码；内部包装为 `Result` |
| `Atomic*` 忘记指定 `Ordering` | 编译错误或数据竞争 | 一致使用 `SeqCst`（最安全）；明确场景用 `Acquire`/`Release` |
| on-flash struct 添加 Rust 专用字段 | 二进制不兼容 | flash struct 只有 `#[repr(C)]` 字段；Rust 专用状态放包装类型中 |
| `loop {}` 内不释放锁 | 死锁 | 确保 `loop` 内有 `break` 或 `return`；锁的 `Drop` 在 `loop` 外 |
| `format!` / `Vec` 在 `no_std` + `no_alloc` | 编译失败 | 用 `heapless::String` + `write!`；`heapless::Vec` 替代 `Vec` |
| `union` 字段访问（Rust < 1.84） | 需要 `unsafe` | Rust 1.84+ 所有字段为 `Copy` 时安全访问；低版本保持 `unsafe` |
| `OnceLock` vs `LazyLock` 语义混淆 | 初始化时机不同 | `OnceLock` 需显式 `get_or_init()`；`LazyLock` 首次访问自动初始化 |
| C `sizeof` 在宏中展开 vs Rust `size_of` | 编译时 vs 运行时 | Rust `core::mem::size_of::<T>()` 是编译时常量；`size_of_val(&t)` 需要值 |
| C 隐式整型提升 vs Rust 无隐式转换 | 编译错误 | C 中 `uint8_t + uint16_t` 隐式提升；Rust 需显式 `as u16` 转换 |

#### B. Rust 易错表

| Rust 注意事项 | 关键提醒 |
|-------------|---------|
| `unsafe` 块中的指针算术 `ptr.add(n)` | 必须确保不越界；优先用切片索引 `slice[n..]` 替代 |
| `ptr::read_volatile` 用于 flash 读取 | C 代码可能缺少 `volatile` 关键字（潜在 bug）；Rust 中必须用 `read_volatile` |
| `zerocopy::FromBytes` 要求类型无填充 | `#[repr(C)]` struct 可能有 padding，需用 `zerocopy::KnownLayout` 或手动序列化 |
| `cfg(feature = "a")` 与 `cfg(feature = "b")` 互斥 | Cargo features 是 additive 的，不能声明互斥；用 `cfg_attr` 或 build.rs 控制 |
| const generic 默认值 `struct S<const N: usize = 64>` | 需 Rust 1.59+；旧版本用 `type alias` 替代 |


### §5 零容忍项

+ ❌ **禁止简化/硬编码/伪造默认值** —— 禁止与翻译前 C 源码不一致的默认值，注释含"简化"/"待实现"/"后续实现"。有困难应向用户提问
+ ❌ **禁止下划线前缀变量名** —— `_` 前缀绕过 unused 检查意味着功能缺失
+ ❌ **禁止不参照翻译前 C 源码修改代码** —— 所有源码修改必须和翻译前 C 源码对照实现，包括 bug 修复、翻译、UT 改动等
+ ❌ **禁止改变 on-flash 二进制格式** —— 所有用于 flash 序列化的 struct 必须保持与 C 版本字节级一致；`#[repr(C)]` + `size_of` 验证是强制要求
+ ❌ **禁止 `unimplemented!()` / `todo!()` 占位** —— 功能遗漏；有困难应向用户提问
+ ❌ **禁止将 `.h` 独立映射为 `.rs` 模块** —— `.h` 是声明文件，不是语义单元。其类型定义必须合并到对应的实现模块（`.c` → `.rs`），公共声明由 `lib.rs` 的 `pub use` 替代；编译时配置头文件（`*_cfg.h`）映射到 `Cargo.toml` features

---

## 强制事项

1. ✅ **每次编码前必须内化 §1–§4**
2. ✅ **翻译复杂模式时先查 references**：翻译表中标注"详见 references/xxx.md"时，必须先加载再翻译（按 Step 3 规则）
3. ✅ **§2 建议项必须向用户提起审阅**，获批准后方可应用
4. ✅ **on-flash struct 必须验证 `size_of`**：涉及 flash 序列化的 struct，翻译后必须用 `assert_eq!(core::mem::size_of::<Struct>(), EXPECTED_SIZE)` 验证布局一致

## 禁止事项

1. ❌ 禁止跳过 §3 禁令表中的任何一项（零容忍）
2. ❌ 禁止在未查 references 的情况下直接处理类型双关/offsetof/条件布局/继承/goto 等复杂模式
3. ❌ 禁止修改 references 文件内容（references 来自业界最佳实践，是基准）
4. ❌ 禁止未经用户同意，直接应用 §2 建议项
5. ❌ 禁止在 on-flash struct 上使用非 `#[repr(C)]` 布局
