# 需求类型模板：migration（跨语言迁移）

---

## §1 元信息

- **模板名**: `migration`
- **适用场景**: 跨语言迁移/翻译类需求（如 Java→Rust）
- **激活条件**: "迁移"、"翻译"

## §2 附加流程环节

跨语言迁移时，**必须在 Plan 生成前完成迁移分析**，确保 `ignore.yaml` 就绪。

### 执行方式：声明调用 `migration-parity-analyzer` Skill

本模板**不包含迁移分析的执行细节**——迁移分析由独立的 `migration-parity-analyzer` Skill 执行，该 Skill 负责：

1. graphify 图谱生成 + 质量验证
2. 逐节点 1:1 审查（7项检查 + 强制 subAgent）
3. 偏离规则分类（mandatory/recommended/incompatible）
4. 翻译 Skill 规则覆盖度反向检查
5. 无法 1:1 的类和函数给出翻译方案，提交人工审查
6. 生成 ignore.yaml

**调用方职责**（`requirement-analysis-helper` 或主 Agent）：
- 加载 `migration-parity-analyzer` Skill
- 等待其完成全部流程并产出 `ignore.yaml`
- 确保 `ignore.yaml` 经人工审阅

### 准入约束

- [ ] `migration-parity-analyzer` Skill 已加载并执行完毕
- [ ] `ignore.yaml` 已生成
- [ ] `ignore.yaml` 中所有 recommended 和 incompatible 项已经人工审阅（逐一核实，非批量确认）

## §3 附加需求拆分策略

（注入位置：§2 拆分策略表 + 详细策略）

### 拆分策略表行

| 需求类型 | 有参考代码? | 拆分主维度 | 估算依据 |
|---------|-------------|-----------|---------|
| **重构 / 迁移类** | ✅ 旧版本或其他语言源码 | 目录 | 参考代码目录行数 |

### 策略：按目录

1. 列出所有涉及的目录（按参考代码目录结构）
2. 估算每目录行数：翻译类≈源文件行数（±15%）；重构类=旧代码 + 调整比例
3. 构建目录依赖图（DAG）：分析 import/include/`mod` 等关系
4. 拓扑排序：被依赖的目录优先实现
5. 拆分：单目录 < 2000 行独立 Plan；多个小目录可合并（合计 ≤ 2000）；单目录超限时分子目录（保持类/文件完整）

## §4 附加输出制品

（注入位置：§5 制品表）

| 制品 | 路径 | 产出阶段 |
|------|------|---------|
| ignore.yaml | `.opencode/harness/manifests/<module>-ignore.yaml` | 迁移审查后（由 `migration-parity-analyzer` 产出） |


## §5 附加 Final Plan 准入条件

（注入位置：§3 Final Plan 准入条件）

- [ ] 迁移类：`ignore.yaml` 就绪

## §6 依赖的外部 Skill

| Skill | 调用阶段 | 职责 |
|-------|---------|------|
| `migration-parity-analyzer` | 分析阶段（Plan 生成前） | 执行完整的迁移对齐分析流程：图谱生成、逐节点审查、偏离分类、规则覆盖度检查、生成 ignore.yaml |
| `java-translate-to-rust`（或对应语言的翻译 Skill） | 由 `migration-parity-analyzer` 在分析阶段调用（仅 §1/§2/§3）+ 代码生产阶段（完整） | 分析阶段：识别偏离项 + 规则覆盖度检查；代码生产阶段：翻译规则依据 |
