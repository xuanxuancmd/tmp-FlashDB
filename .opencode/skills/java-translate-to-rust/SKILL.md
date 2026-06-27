---
name: java-translate-to-rust
description: Java 到 Rust 1:1 代码翻译实战指南。提供翻译映射速查表、Rust 禁令表、翻译易错表，以及各种复杂场景的详细参考方案。
---

# Java→Rust 翻译实战指南

## 职责

本 Skill 为 AI 编码 Agent 提供 Java→Rust 代码翻译的规则与决策框架。

## 使用前必读

+ 不在本文描述的 Java→Rust 规则通常较为简单，模型可以自主决策。
+ 从上下文或 AGENTS.md 中查询 Java 源码的位置，若未找到应通过 `question()` 向用户询问。
+ 翻译默认遵从 1:1 原则，确保规格不遗漏，但同时也需遵从 Rust 语言特点。

## 工作流程

### Step 1：翻译基本原则

+ **1:1 翻译约束**：代码翻译保证目录、文件数、类、函数名 1:1 对齐（采用 Rust snake_case 命名风格）；语言差异过大无法 1:1 映射时，需重点分析迁移方案并提交用户审阅
+ **目录映射**：Java 类路径去除公共父路径 → 保留相对路径。如 `org/apache/kafka/connect/connector/ConnectRecord.java` → `connector/connect_record.rs`
+ **文件映射**：Java 独立 class（内部类除外）→ Rust 独立 `.rs` 文件，不能合并多个 struct

### Step 2：加载翻译基线（每次翻译前必读）

"核心翻译规则"章节的三张表（§1 强制翻译表、§2 建议翻译表、§3 禁令表、§4 易错表）是翻译的规则手册，任何涉及 Java→Rust 转换的编码前必须内化。

**注意：尤其是上述表中第一列，起到目录导航作用**

### Step 3：翻译中按需加载 references

+ 简单场景，通过翻译表直接查看，对应章节§1-A
+ 复杂场景，对应章节§1-B，翻译表中的场景行会标注"详见 `references/xxx.md`"，如果java场景匹配到复杂场景后，就需要加载对应 reference 文件，读取完整方案后再动手翻译。

### Step 4：翻译后必查

+ **易错检查**：对照 §3 翻译易错表
+ **禁令验证**：对照 §2 Rust 禁令表，确保无违禁模式
+ **与源码对照**：所有修改必须与 Java 源码对照，确保 1:1 映射

---

## 核心翻译规则

### §1 强制翻译表 [强制应用]

#### A. 简单规则 — 直接应用

| Java 特性 | Rust 替代 | 迁移规则 | 代码举例 |
|----------|----------|---------|---------|
| 方法重载 | 泛型 + trait 约束，或 `<fn>_by_<xx>` 命名 | 参数类型相似 → 泛型统一签名；参数语义不同 → 独立函数名 | `fn process<T: Into<String>>(input: T)` / `fn schema_by_value() + fn schema_by_class()` |
| Java 静态内部类 | 平铺独立 struct | 独立 struct，命名 `<父类名><当前类名>`（snake_case） | `class Outer.Inner`（static）→ `struct OuterInner { ... }` |
| `ConcurrentHashMap` | `DashMap<K, V>` | `get()` 返回带锁 `Ref`，**必须在 await 前释放**（获取后立即 clone 值）；`entry().or_insert_with()` 闭包内禁止再访问同 map（会死锁）；迭代禁止 `remove()` | `match map.get("k") { Some(g) => { let v = g.clone(); drop(g); async_fn(v).await; } _ => {} }` |
| `ConcurrentLinkedQueue` | `RwLock<VecDeque<T>>` | `offer()` → `.write().await.push_back()`；`poll()` → `.write().await.pop_front()` | `queue.write().await.push_back(item)` |
| Java 泛型声明（类型参数、约束、通配符） | Rust 泛型 + trait bounds | `<T extends Foo>` → `<T: Foo + Send + Sync + 'static>`；通配符 `?`/`Class<?>` → `dyn Trait`；泛型接口 → 泛型 trait；泛型子类特化 → 组合 + 具体参数 | `struct Table<R: Eq + Hash, C: Eq + Hash, V: Clone>` / `Arc<dyn KafkaProducer>` / `PhantomData<R>` |
| volatile | Atomic* 类型 | `volatile int` → `AtomicI32`，**强制 `Ordering::SeqCst`** | `AtomicI32::new(0).load(Ordering::SeqCst)` |
| ExecutorService | tokio runtime | `submit()` → `tokio::spawn()` | `tokio::spawn(async { worker.start() })` |
| 块级synchronized（非函数签名） | `tokio::sync::Mutex<T>` | 方法代码块内 `let guard = self.xxx.lock().await` | `{let guard = self.state.lock().await; ...}` |
| Singleton | `once_cell::sync::Lazy` | `static INSTANCE` → 惰性静态初始化 | `static METRICS: Lazy<Metrics> = Lazy::new(Metrics::new);` |
| Thread context ClassLoader | `thread_local!` | `Thread.currentThread().setContextClassLoader()` → 线程局部存储 | `thread_local! { static PLUGIN_ID: RefCell<Option<String>> = const { RefCell::new(None) }; }` ← 线程局部可变状态需 RefCell 或 Mutex |
| CompletableFuture | oneshot channel 封装兼容 API | `codes/completable_future.rs` |
| CountDownLatch | AtomicUsize + AtomicWaker 封装 | `codes/countdown_latch.rs` |
| Java 非静态内部类（实现标准函数接口 Runnable / Callable / Consumer / Supplier） | 直接翻译为 Rust 闭包 | 闭包按 `move` 值捕获外部字段，等价于 Java 内部类隐式持有 `this`；无需独立 struct。闭包过长（>30 行）才提取为命名函数 | `exec.spawn({ let field = self.field.clone(); async move { field.do_xxx().await } });` |

#### B. 复杂规则 — 先查 references 再翻译

| Java 特性 | Java 示例 | Rust 映射规则 | rust参考示例 |
|----------|----------|--------------|---------|
| 子类型固定、同目录的继承体系（异常/状态枚举） | — | Enum + match：**封闭类型体系** | `enum ConnectError { Data { msg: String }, Config { msg: String } }` — 详见 [inheritance-enum-trait.md](references/inheritance-enum-trait.md) §1.1 |
| 纯行为抽象类（无状态字段） | — | trait（可含默认方法）：**纯行为抽象** | — |
| 抽象类继承 | — | trait + getter：抽象类的属性抽象成struct，trait中定义getter 接口，子类实现并返回该struct | 详见 [inheritance-trait-getter.md](references/inheritance-trait-getter.md) |
| 实体类继承 | `class B extends A {}`（A 是实体类） | 组合 + trait: `struct Child { base: Parent }`，子 struct 持有父 struct 作为字段，通过 `self.base.xxx()` 委托访问；抽取模板方法为Hooks trait，从而支持多态（可选）。 | 详见 [inheritance-concrete-composition.md](references/inheritance-concrete-composition.md)（场景 A/B） |
| 多级类继承 | 多级继承 `A←B←C` | **多级继承 `A←B←C`**：A 抽象类无状态 → `trait A`；B 实体类继承 A → `struct B impl A`（flat impl trait A）；C 实体类继承 B → `struct C { base: B } impl A`（trait A 方法全部委托到 `self.base.xxx()`） | 多级继承参考“抽象类继承”+“实体类”叠加场景 |
| 监听器回调中 this 反向传递导致循环引用 | `A.store.setListener(this)`：A 持有 Store，Store 又持有 Listener（=A），Rust 中形成 `Arc` 环 | 适配器模式（Adapter + channel 解耦） | Adapter struct 提取最小依赖字段（**禁止持有** `Arc<ParentType>`），channel 转发异步回调 — 详见 [callback-cyclic.md](references/callback-cyclic.md) |
| Java 非静态内部类（数据 / 逻辑辅助，注册给外部） | `class Worker { int count; class Helper { void inc() { count++; } } }` | 独立 struct + **仅提取内部类实际访问的外部字段**为 `Arc<依赖>`；外部和内部共用同一份 Arc。**禁止**持有任何形式的 `Arc<OuterType>` / `Weak<OuterType>` | `struct WorkerHelper { count: Arc<AtomicI32> }` — 详见 [inner-class-helper.md](references/inner-class-helper.md) |
| Java 非静态内部类（实现 Listener / Callback 接口，注册给外部组件） | `herder.store.setListener(new ConfigListener() { public void onUpdate(...) { /* 引用 herder 字段 */ }})` | Adapter struct + **仅提取最小依赖** `Arc<字段>`（**禁止** `Arc<Herder>`）；异步解耦用 channel。三角循环（Herder ↔ Store ↔ Listener）详见参考文档 | `struct HerderConfigAdapter { generation: Arc<AtomicI32> }` — 详见 [callback-cyclic.md](references/callback-cyclic.md) §8.1–§8.3（4 种子模式） |
| Java `instanceof` 类型判别 | — | trait default 方法 + `new_arc()` + Weak 自引用 | 在基 trait 增加返回 `Option` 的 default 方法，目标子类返回 `Some`，其余返回 `None`，替代运行时类型检查 — 详见 [instanceof-generic.md](references/instanceof-generic.md) |


### §2 建议翻译表 [提交用户审阅后采用]

以下映射涉及架构决策，**不可直接应用**，必须向用户说明并获审阅。

**注意：如果涉及如下“Java特性”，请使用“提问说明”向用户提问。如果接受建议方案，则读取详细参考。**

| Java 特性 | Rust 建议方案 | 提问说明 | 详细参考 |
|----------|-------------|---------|---------|
| SPI（ServiceLoader.load） | 编译时插件注册（inventory crate） | "Java SPI 运行时扫描在 Rust 中需改为**编译时注册**。是否接受引入 `inventory` crate 作为插件发现机制？" | [reflection-spi.md](references/reflection-spi.md) - 1.3章节 |
| 反射（Class.forName / newInstance） | 编译时注册（同上） | "同上。Rust **无运行时反射**，`Class.forName()` 全部需改为编译时注册。是否接受该退化？" | [reflection-spi.md](references/reflection-spi.md) |
| ClassLoader（类隔离/热加载） | 编译时链接 + 配置驱动工厂 | "你的 ClassLoader 是 **(A) 类隔离** 还是 **(B) 热加载**？A → 删除 ClassLoader；B → 引入 `libloading` 动态库。" | [classloader.md](references/classloader.md)  - 1.2章节|
| 函数级 synchronized | 内部锁 `Mutex<()>` 每个方法入口处上锁 | "便于 1:1 对照；但**非 Rust 最佳实践**。是否接受？或希望重构为：① 拆方法避免重入 ② 提取 Atomic 计数器 ③ channel/actor 模式？" | — |

### §3 禁令表 [必读·零容忍]

**如下语言级禁令，除非用户许可否则禁止使用：**

| 禁止事项 | 原因 |
|---------|------|
| 对 Result 变量使用 `unwrap()` 或 `expect()` | 可能导致进程 panic |
| Java 独立 class 对应多个 struct 合并同一 `.rs` 文件 | 破坏 1:1 映射，代码不可追踪 |
| `std::sync::Mutex` / `std::sync::RwLock` | 应使用 `tokio::sync` 替代，适配异步行为 |
| `block_in_place` / `block_on` / `blocking_lock` / `blocking_read` / `blocking_write` | 可能导致线程耗尽从而死锁 |
| `Mutex<primitive>`（如 `Mutex<u64>`、`Mutex<bool>`） | 应使用 Atomic* 代替，性能更安全 |
| `log!` 宏内部使用 `await` | log 宏不支持非 Send 代码 |
| `try_lock`（非阻塞锁获取） | 除 `std::fmt::Debug` 实现外，其余地方禁止使用 |

**翻译过程禁令：**

+ ❌ async 函数调用处**必须有 await**，尤其是无返回值的 async 函数容易遗漏，无 await 导致函数不执行，功能缺失

**禁令例外：**
+ std::fmt::Debug实现中允许使用try_lock，避免使用async、await的编译报错

### §4 易错表 [必读]

#### A. Java -> Rust 映射易错表
| Java 特性 | Rust 注意事项 | 关键提醒 |
|----------|-------------|---------|
| 多个 `public static void main()` | Rust 需要在 struct 外定义全局 main 入口 | 不同于 Java 每个 class 都可以有 main |
| 同步方法 | async + await 行为上等同同步调用，**禁止使用blocking_on操作** | 确保 1:1 对应，不丢失调用链 |
| 阻塞锁 | **禁止 `try_lock` 非阻塞**，应用 `lock().await` 阻塞等待，除非Java是非阻塞锁 | `try_lock` 仅允许在 `std::fmt::Debug` 实现中 |
| 内部类 / Listener 捕获外部 `this` | downgrade 来源必须是长期存活的 `Arc<Mutex<T>>`，禁止从临时 Arc 创建 | `Arc::try_unwrap()` 消费临时 Arc 后，从它 downgrade 的 Weak 悬垂，后续 `upgrade()` 返回 None，回调静默丢失——难排查 |
| 内部类捕获外部 `this`，但外部对象生命周期更短 | 两种情况不适合 Weak：① 持有 Weak 的 Arc 在构造中被 `try_unwrap` 消费；② 回调语义要求目标必须存活 | 替代：从大对象提取**最小依赖字段**构造独立 Adapter，使生命周期独立 |
| `toString()` 方法 | 不要直接翻译 `toString()`，改用 `impl std::fmt::Debug` | Rust 习惯用 Debug trait，不用 Display |

#### B. Rust易错表
| Rust 注意事项 | 关键提醒 |
|-------------|---------|
| asyn函数调用处**必须有 await**，否则函数根本不执行 | 无 await → 函数不执行，**功能缺失**且编译器不报错 |
| 内部类 / Listener 捕获外部 `this` | downgrade 来源必须是长期存活的 `Arc<Mutex<T>>`，禁止从临时 Arc 创建 | `Arc::try_unwrap()` 消费临时 Arc 后，从它 downgrade 的 Weak 悬垂，后续 `upgrade()` 返回 None，回调静默丢失——难排查 |
| 内部类捕获外部 `this`，但外部对象生命周期更短 | 两种情况不适合 Weak：① 持有 Weak 的 Arc 在构造中被 `try_unwrap` 消费；② 回调语义要求目标必须存活 | 替代：从大对象提取**最小依赖字段**构造独立 Adapter，使生命周期独立 |

### §5 零容忍项

+ ❌ **禁止简化/硬编码/伪造默认值** —— 禁止与翻译前源码不一致的默认值，注释含"简化"/"待实现"/"后续实现"。有困难应向用户提问
+ ❌ **禁止下划线前缀变量名** —— `_` 前缀绕过 unused 检查意味着功能缺失
+ ❌ **禁止不参照翻译前源码修改代码** —— 所有源码修改必须和翻译前源码对照实现，包括 bug 修复、翻译、UT 改动等

---

## 强制事项

1. ✅ **每次编码前必须内化 §1–§4**
2. ✅ **翻译复杂模式时先查 references**：翻译表中标注"详见 references/xxx.md"时，必须先加载再翻译（按 Step 3 规则）
3. ✅ **§2 建议项必须向用户提起审阅**，获批准后方可应用

## 禁止事项

1. ❌ 禁止跳过 §3 禁令表中的任何一项（零容忍）
2. ❌ 禁止在未查 references 的情况下直接处理继承/反射/回调等复杂模式
3. ❌ 禁止修改 references 文件内容（references 来自实际翻译经验，是基准）
4. ❌ 禁止未经用户同意，直接应用 §2 建议项

