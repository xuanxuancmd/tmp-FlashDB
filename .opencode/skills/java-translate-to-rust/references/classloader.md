> **导航**: ← [反射与SPI](./reflection-spi.md) | [SKILL.md](../SKILL.md) | → [接口回调与循环引用](./callback-cyclic.md)

---

## 1. 自定义ClassLoader

**核心差异**：Java ClassLoader提供运行时类隔离和动态加载，Rust编译时链接无此机制，需采用不同策略替代。

### 1.1 ClassLoader用途翻译方案

| Java ClassLoader用途 | Rust替代 | 适用场景 |
|--------------------|----------|---------|
| 插件隔离（不同版本共存） | 退化方案：编译时动态集成（参考1.2） | “由用户选择” |
| 热加载（动态替换类） | 方案1：退化方案：不提供热加载<br>方案2：libloading动态库 | “由用户选择” |
| DelegatingClassLoader | 配置驱动工厂 | 插件查找逻辑 |

### 1.2 运行时动态加载退化方案：编译时动态集成（推荐）

**核心思路**：Rust编译时链接所有插件，无运行时隔离需求。插件冲突通过Cargo版本管理解决，编译时类型检查比ClassLoader隔离更安全。

**代码片段**：
```rust
// 所有插件编译时注册
inventory::submit! { PluginEntry { name: "FileStreamSourceConnector", factory: ... } }

// PluginManager统一查找
pub struct PluginManager {
    plugins: HashMap<&'static str, fn() -> Box<dyn Connector>>,
}

impl PluginManager {
    pub fn load(&self, name: &str) -> Option<Box<dyn Connector>> {
        self.plugins.get(name).map(|f| f())
    }
}
```

### 1.3 配置驱动工厂替代ClassLoader查找

**核心思路**：Java ClassLoader根据类名查找jar包并加载类，Rust通过配置文件映射类名到工厂名称，运行时通过工厂注册表查找。

**代码片段**：
```rust
// 配置驱动查找
pub struct PluginConfig {
    plugin_mappings: HashMap<String, String>,  // className -> factoryName
}

impl PluginManager {
    pub fn load_by_config(&self, config: &PluginConfig, class_name: &str) -> Option<Box<dyn Connector>> {
        let factory_name = config.plugin_mappings.get(class_name)?;
        self.plugins.get(factory_name).map(|f| f())
    }
}

```
