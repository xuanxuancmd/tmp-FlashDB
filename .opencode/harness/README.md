# Kafka Connect Harness

## 概述

本 Harness 系统用于确保 Kafka Connect 模块从 Java 到 Rust 的 **1:1 重写无遗漏**。它提供三层防护：
- **黄金清单约束**：定义必须翻译的所有类、方法
- **Placeholder零容忍**：禁止AI留下未实现的代码
- **校验闭环**：编码后必须通过三层校验才能声称"完成"

## 目录结构

```
.opencode/harness/
├── README.md                     # 本文件，使用说明
├── manifests/
│   └── *.golden.yaml             # 黄金清单（工具生成，经过人工审阅后允许AI编辑）
├── ignores/
│   ├── language-ignores/
│   │   └── java-rust.yaml        # Java→Rust 通用 ignore 规则（内置）
│   └── {module}-ignores.yaml     # 模块级 ignore 规则（AI 申请后累积）
├── features/
│   └── *.feature                 # BDD场景定义（Plan Agent输入）
├── scripts/
│   ├── build_golden_manifest.py  # 从Java源码生成黄金清单
│   ├── verify_manifest_parity.py # 校验Rust实现与清单一致性
│   ├── detect_placeholders.py    # 检测占位代码
│   └── compare_graphs.py         # 图谱比对：调用链/接口实现/依赖一致性
└── evidence/
    └── *.json                    # 校验输出产物
```

## 执行流程

### 编码前（需求分析阶段）

**如下内容在首次迁移该模块时，应该自动触发执行：**

1. 黄金清单（完整性基准）

   从 Java 源码提取所有类/方法/字段，定义 Rust 必须实现的完整清单，生成到`harness/manifests/<module-name>*.golden.yaml`中。

   **约束**：

   - 由 `build_golden_manifest.py` 工具生成
   - 黄金清单文件内容禁止删减
   - 翻译新模块前必须生成
   - Plan Agent 生成执行计划时引用

   **生成黄金清单方法**：

   ```bash
   python build_golden_manifest.py
     --module <module-name> \
     --java-root connect/<module-name>/src
   ```
   生成的清单位于：`.opencode/harness/manifests/<module-name>*.golden.yaml`

**黄金清单编辑约束：**

黄金清单的修改仅限于追加 `ignore` 标注，且必须经过用户同意。ignore标注有两种触发时机：

1. **需求分析/规划阶段**：识别无法1:1映射的场景，提前标注ignore
2. **检视修复阶段**：对Category A（语言差异型）parity issue申请ignore标注（分类规则详见 `harness-code-review` Skill）

两种时机均必须经过用户审批，主Agent不得擅自修改黄金清单。Category B（可修复型）issue不允许标注ignore，必须修复代码。

**ignore 格式参考**：两种触发时机在生成 ignore 规则前，都必须先 `Read .opencode/harness/ignores/ignore-template.yaml` 了解格式规范。

   （注意：*.golden.yaml文件可能较大应该分段读入）

   - **禁止删除黄金清单中任意条目**
   - **唯一可修改前提**：Java和Rust语言差异导致无法1:1映射（如ClassLoader→宏、异常继承→enum、内部类→平铺），经用户同意后追加ignore

2. BDD 规格（正确性基准）
   探索当前模块的测试场景（可通过deepwiki工具辅助生成），遵从Cucumber/Gherkin 风格测试场景，并生成到`harness/features/<module-name>-*.feature`。

   **用途**：

   - Plan Agent 生成执行计划时引用
   - 每个任务标注对应 BDD 场景 ID
   
   注意：
   
   + BDD的测试场景可以是分层测试，但必须包含多个E2E的测试用例。

### 编码中

AI Agent 按照带清单的 Plan 执行编码：
- 每个子任务完成后运行 `cargo check`
- 将符合`.opencode/harness/features/<module-name>*.feature`文件拷贝到当前模块的tests/resources目录下，通过cucumber-rs组件实现对应BDD测试用例。
- Hook自动检测placeholder代码

### 编码后
#### 完整性校验
1. 校验黄金清单

```bash
   python verify_manifest_parity.py
     --module <module-name> \
     --rust-root connect-rust/connect-<module-name>/src
    ```
    生成的issue报告位于：`.opencode/harness/evidence/<module-name>*-parity.json`

2. 校验代码无空实现、todo等

```bash
    python detect_placeholders.py
      --module <module-name> \
      --rust-root connect-rust/connect-<module-name>/src
    ```
    生成的issue报告位于：`.opencode/harness/evidence/<module-name>-placeholder.json`

3. 图谱比对（调用链/接口实现/依赖一致性）

```bash
    python compare_graphs.py
      --source-graph <java-source-dir>/graphify-out/graph.json \
      --target-graph connect-rust/connect-<module-name>/src/graphify-out/graph.json \
      --source-language java \
      --target-language rust \
      --filter-crate connect-<module-name> \
      --ignores .opencode/harness/ignores/<module-name>-ignores.yaml \
      --output .opencode/harness/evidence/<module-name>-graph-parity.json
    ```
    生成的issue报告位于：`.opencode/harness/evidence/<module-name>-graph-parity.json`

    **比对范围**（仅比对以下 4 种边类型）：
    - `contains`：目录/文件/类是否存在（对应 Java 的 class/interface/enum 文件）
    - `method`：函数是否存在（对应 Java 的方法声明）
    - `implements`：继承/接口实现是否完整（对应 Java 的 implements/extends）
    - `calls`：调用链是否一致（对应 Java 的方法调用）

    **不比对的边类型**：`references`（类型引用如 String/Map）、`imports_from`（模块导入）——这些是语言差异，不是翻译完整性问题。

    **说明**：
    - 源码图谱（Java）只需建立一次（graphify），重复运行时检查是否已存在
    - 目标代码图谱由 graphify `--watch` 模式自动刷新（hooks 驱动）
    - `--filter-crate` 控制比对范围（module/directory/project），由 AI Agent 从上下文自动推断
    - ignore 文件分两层：`language-ignores/` 内置通用规则（脚本自动加载） + `{module}-ignores.yaml` 模块级规则（AI 发现语言差异后向用户申请，追加写入）

#### Ignore 文件格式

ignore 文件基于 **Java 全路径（包名+类名）**，支持 4 种 ignore 类型。AI 参考 `ignores/ignore-template.yaml` 模板生成。

```yaml
ignores:
  # === 1. 类级 === 整个类无法 1:1 翻译
  - class: org.apache.kafka.connect.converters.BooleanConverter
    reason: "Java抽象类继承，Rust用trait组合"

  # 内部类：用 OuterClass.InnerClass
  - class: org.apache.kafka.connect.runtime.Worker.TaskBuilder
    reason: "Java Builder模式内部类"

  # === 2. 方法级 === 类可翻译，但某个方法无法 1:1
  - class: org.apache.kafka.connect.cli.AbstractConnectCli
    method: startConnect
    reason: "Java内部方法命名转换"

  # === 3. 调用链级 === 特定的函数调用无法翻译
  - from_class: org.apache.kafka.connect.runtime.WorkerSourceTask
    from_method: run
    to_class: org.apache.kafka.connect.runtime.WorkerSourceTask
    to_method: validate
    reason: "Java异常处理机制差异，Rust使用Result替代"

  # === 4. 继承级 === 继承关系无法 1:1 翻译
  - class: org.apache.kafka.connect.converters.BooleanConverter
    inherits: org.apache.kafka.connect.storage.Converter
    reason: "Java抽象类继承，Rust用trait组合替代"
```

**字段说明**：
- `class`：Java 全路径（包名+类名，必填）—— 脚本提取最后一段类名匹配图谱节点（支持 `*` 通配）
- `method`：Java 方法名（可选）—— 仅匹配该类的指定方法（支持 `*` 通配）
- `inherits`：父类/接口全路径（可选）—— 仅匹配 `implements` 边
- `from_class/from_method` + `to_class/to_method`：调用链级 ignore
- `reason`：ignore 原因（必填，人类可读）

**匹配精度**：提供的字段越多，匹配越精确。仅 `class` → 匹配涉及该类的所有边；加 `method` → 仅匹配指定方法；加 `inherits` → 仅匹配继承边。

**生成流程**：
1. **Read `.opencode/harness/ignores/ignore-template.yaml`**（强制，了解格式规范）
2. AI Agent 在图谱比对或需求分析后发现语言差异
3. 按模板格式生成 ignore 规则，选择 ignore 类型
4. 向用户申请确认（question()）
5. 用户同意后，追加到 `.opencode/harness/ignores/{module}-ignores.yaml`
6. 下次图谱比对时自动生效

**两层 ignore 机制**：
- `language-ignores/java-rust.yaml`：内置通用规则（Rust 标准 trait 方法等），脚本自动加载，无需申请
- `{module}-ignores.yaml`：模块级规则（架构差异、框架映射等），需用户审批后追加

**从黄金清单批量提取**：
```bash
python convert_manifest_ignores.py \
  .opencode/harness/manifests/{module}.golden.yaml \
  .opencode/harness/ignores/{module}-ignores.yaml
```

#### Parity校验差异分类策略

parity issue分为两类：**Category A**（语言差异型，无法1:1映射，经用户审批后可标注ignore）和 **Category B**（可修复型，必须修复代码）。详细判定条件、修复策略和交叉决策矩阵 → 详见 `harness-code-review` Skill。

#### 正确性校验
##### 检测内容

1. 校验结构完整性

2. 检测是否存在空实现

3. 运行测试

   运行测试（含cucumber测试），用例运行失败则修复源码
   `cargo test --workspace`

##### 检测时机

**1. 增量检测（实时阻断）**

**触发**：编辑 Rust 文件后立即执行

**检测项**：

- placeholder 代码 → **阻断**（必须立即修复）
- 一致性缺失 → **提示**（全量检测时统一处理）

**2. 全量检测（完整验证）**

**触发**：用户停止交互后自动执行

**检测项**：
- placeholder 全目录检测 → 阻断
- manifest parity 校验 → 阻断（缺失项）
- cargo check → 阻断（编译错误）

**输出**：`evidence/*.json`（供 AI 修复参考）

## 黄金清单说明

黄金清单是翻译的"源标准"，由工具从Java源码生成，包含：
- 模块名、Java根目录、Rust根目录
- 所有Java类、接口、枚举
- 方法列表（名称、参数数量、返回类型）
- 预期的Rust文件名映射
- Java->Rust的重点语法映射

**重要约束**：

- 黄金清单由 `build_golden_manifest.py` 工具生成
- **禁止** AI Agent 擅自修改黄金清单（仅可在用户审批通过后追加ignore标注）
- 清单必须与 Java 源码同步

## BDD说明

+ 禁止在编码阶段修改BDD的feature文件
+ 为feature文件实现测试用例时（tests目录下），禁止对connect目录内其他模块代码打桩，只允许对于外部网络请求等打桩。
+ 在编码及编码后的阶段中，BDD测试用例运行失败，需要反馈并自行修复代码问题。

## 常见问题

**Q: 校验失败怎么办？**
A: 查看 `evidence/*.json` 中的 `missing_items` 和 `placeholder_items`，按优先级补齐。AI Agent应消费这些证据文件进行修复。

**Q: BDD规格什么时候用？**
A: Plan Agent生成执行计划时读取 `features/*.feature`，每个任务应标注对应的BDD场景ID。

---

*维护者: 项目架构师*