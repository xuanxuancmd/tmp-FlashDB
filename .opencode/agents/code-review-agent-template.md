# 检视规则模板

> 本文件是 code-review-agent 的**项目级规则配置**。Agent 执行 Phase 4.2 零容忍检视时，必须 `Read` 本文件获取具体检查项。
> 项目初始化时按需修改本文件，使之匹配当前项目的编码规范。

---

## §1 零容忍规则

以下规则违反即标注 **HIGH** 优先级，阻断检视通过。

### R-01: 进程崩溃调用

禁止使用会导致进程 panic / crash 的 API 调用。

**违规判定**：代码中（非 `#[test]` 标注的函数体内）出现以下模式之一即视为违规：

- `unwrap()`
- `expect()`
- `panic!()`
- `unreachable!()`
- `todo!()`

**例外**：`#[test]`、`#[cfg(test)]` 模块内允许。

### R-02: 未实现代码 / 占位符

禁止提交含未实现逻辑的代码。

**违规判定**（任一命中即违规）：
- 函数体为空或仅含 `return`/`Ok(())` 且注释含 TODO/FIXME/HACK
- 函数体仅返回默认值 / 假值（如 `false`、`0`、`""`、`Vec::new()`）而无实际逻辑
- 编译期强制 lint 声明缺失（见 §3 编译期 lint 清单）

**注释脱节检测**：
- 注释中出现"简化"、"待实现"、"后续实现"、"暂时"、"临时"等字样 → 若对应函数体确无实现 → HIGH
- 注释中出现"简化"等字样 → 若函数体有实现但实现不完整（与注释描述不符）→ MEDIUM

### R-03: 硬编码 / 伪造默认值

禁止与规格/需求不一致的默认值或硬编码。

**违规判定**：
- 函数参数被忽略，使用硬编码默认值替代（如超时写死 `30000ms`、重试次数写死 `3`）
- 魔术数字（magic number）无注释说明来源
- 字符串硬编码（URL、配置 key 等）且无法在需求文档中找到出处

**例外**：广泛接受的常量（如 `HTTP_200 = 200`），且有常量声明。

### R-04: 变量命名逃逸

禁止用下划线前缀变量名绕过编译器的 unused 警告。

**违规判定**：`_variable_name` 形式的变量，且该变量在函数体内确实未被使用 → HIGH（意味着功能缺失）。

### R-05: 测试关键字侵入正式代码

正式代码（非测试目录/文件）中禁止使用 mock / stub / test 等测试关键字。

**违规判定**：正式源码中出现 `mock`、`stub`、`fake`、`test_only` 等关键字（大小写不敏感）。

**例外**：以下辅助包 / 目录豁免（项目特定，按需配置）：
- `common-trait`
- `kafka-clients-embedded`
- `tests/` 目录
- `benches/` 目录

---

## §2 额外检视项

以下检查项在标准 Phase 1-4.2 之外**额外执行**。违反标注对应严重度。

### E-01: 编译期 deny lints 声明 [HIGH]

**规则**：所有 crate 的 `lib.rs` 顶部必须包含以下 deny 指令

```rust
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::panic)]
#![deny(clippy::unreachable)]
#![deny(clippy::dbg_macro)]
```

### E-02: 外部依赖白名单 [MEDIUM]

**规则**：`Cargo.toml` 中仅允许使用白名单内的外部依赖。

**白名单**：`tokio`, `serde`, `serde_json`, `futures`, `async-trait`, `anyhow`, `thiserror`, `base64`, `regex`, `once_cell`, `chrono`

### E-03: 错误处理完整性 [HIGH]

**规则**：所有可能失败的 IO/网络/序列化操作必须有显式错误处理，不得静默吞掉错误。

---
