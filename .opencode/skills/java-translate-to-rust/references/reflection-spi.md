> **导航**: ← [并发与异步](./concurrency.md) | [SKILL.md](../SKILL.md) | → [ClassLoader](./classloader.md)

---

## 6. 反射与SPI机制

**核心挑战**：Rust无运行时反射，Java的`Class.forName()`、`ServiceLoader.load()`等反射API无法直接翻译，需采用编译时注册或配置驱动工厂替代。

### 6.1 反射翻译方案对照

| Java反射特性 | Rust替代 | 关键差异 |
|------------|----------|---------|
| `Class.forName(className)` | 编译时注册（inventory宏） | 无运行时动态加载 |
| `clazz.newInstance()` | 工厂函数：`fn() -> Box<dyn Trait>` | 无运行时构造 |
| `ServiceLoader.load(Xxx.class)` | `inventory::collect!(XxxEntry)` + 查找 | 无运行时扫描 |
| `Utils.newParameterizedInstance()` | 配置驱动工厂注册 | 无反射创建 |
| `Method.invoke()` | trait方法调用 | 无动态方法调用 |

### 6.2 编译时注册替代运行时反射

**核心思路**：Java通过`Class.forName()`动态加载类并实例化，Rust通过`inventory`宏在编译时收集所有插件注册信息，运行时查找工厂函数创建实例。

**关键步骤**：
1. 定义插件注册条目结构：`PluginEntry { name: &'static str, factory: fn() -> Box<dyn Trait> }`
2. 每个插件模块编译时注册：`inventory::submit!(PluginEntry { name: "...", factory: ... })`
3. 插件管理器运行时查找：遍历`inventory::iter::<PluginEntry>`构建工厂映射表

**代码片段**：
```rust
// 定义注册条目
pub struct PluginEntry {
    name: &'static str,
    factory: fn() -> Box<dyn Connector>,
}

inventory::collect!(PluginEntry);  // 编译时收集

// 插件注册（每个模块）
inventory::submit! {
    PluginEntry {
        name: "FileStreamSourceConnector",
        factory: || Box::new(FileStreamSourceConnector::new()),
    }
}

// 运行时查找
let mut plugins = HashMap::new();
for entry in inventory::iter::<PluginEntry> {
    plugins.insert(entry.name, entry.factory);
}
```

### 6.3 ServiceLoader SPI替代

**核心思路**：Java通过`ServiceLoader`扫描`META-INF/services`目录自动发现实现类，Rust通过`inventory::collect!`编译时聚合所有注册，替代运行时扫描。

**代码片段**：
```rust
// 替代ServiceLoader.load()
inventory::collect!(PluginEntry);

// 替代ServiceLoader遍历
impl PluginManager {
    pub fn discover_all(&self) -> Vec<Box<dyn Connector>> {
        inventory::iter::<PluginEntry>
            .map(|entry| entry.factory())
            .collect()
    }
}
```

### 6.4 Configurable接口配置传递

**核心思路**：Java反射创建实例后检查`instanceof Configurable`再调用`configure()`，Rust在工厂函数中直接包含配置逻辑，避免运行时类型检查。

**代码片段**：
```rust
// 工厂包含配置逻辑
pub struct PluginEntry {
    name: &'static str,
    factory: fn(&HashMap<String, String>) -> Box<dyn Connector>,
}

inventory::submit! {
    PluginEntry {
        name: "DefaultReplicationPolicy",
        factory: |props| {
            let mut policy = DefaultReplicationPolicy::new();
            policy.configure(props);
            Box::new(policy)
        },
    }
}
```
