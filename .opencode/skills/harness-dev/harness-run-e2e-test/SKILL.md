---
name: harness-run-e2e-test
description: E2E验证技能。通过声明式YAML测试规格驱动PowerShell/Bash执行器，完成单微服务的端到端测试闭环，生成带优先级分类的结构化evidence供subAgent做阻断判定和自动修复。
---

# Harness E2E Test Skill (声明式YAML驱动的E2E端到端验证)

## 职责

本Skill指导Agent完成端到端**验证的协调工作**（支持 REST API 和 Shell Commands 两种场景模式）：

1. **确定YAML测试规格**：从调用上下文获取YAML路径，默认 `.opencode/harness/e2e/run-e2e-test.yaml`
2. **环境检测**：自动判断当前运行环境（PowerShell / WSL Bash）并选择对应执行器
3. **调用执行器脚本**：传入YAML文件路径和模块名，启动测试执行
4. **消费evidence**：读取生成的JSON evidence，解读优先级分类和阻断状态

**职责边界**：
- ✅ **协调层**：调用脚本 + 解读结果
- ❌ **不承担**：YAML格式定义（由 `generate-e2e-test-guide` skill 负责）、测试执行逻辑（由 `run-e2e-test.ps1/.sh` 脚本负责）

---

## YAML 格式参考

**YAML 文件格式的权威定义不在本 Skill 中**，请参考：
- 📖 [`generate-e2e-test-guide/SKILL.md`](../generate-e2e-test-guide/SKILL.md) - YAML 生成规范
- 📖 [`generate-e2e-test-guide/templates/scenario-template.yaml`](../generate-e2e-test-guide/templates/scenario-template.yaml) - 字段定义权威

脚本支持两种场景模式（REST / Commands 可混用），执行顺序 = YAML 定义顺序。

---

## `${seq_id}` 自增计数器 （可选）

该字段用于保证每次从头执行e2e/*.yaml自动化用例前，维护并自增该值，并传递给ps1或sh脚本，以保证每次用例用差异，避免部分用例无法可重入的场景出错。

具体操作：

1. 将命令中所有 `${seq_id}` 替换为当前计数器值
2. 每次替换后在当前session内，计数器自增

**替换范围**：所有command，比如`startup_command`（含数组元素），`build_command`、`when.commands`、`cleanup.commands` 等。

---

## 输入参数

Agent 需要根据以下参数与用户交互或从上下文推断来填充值：

| 参数 | 说明 | 值来源 |
|------|------|--------|
| `yaml_spec` | YAML测试规格文件路径 | **上下文优先**（见下方规则），禁止硬编码 |
| `module` | 模块名（用于evidence文件命名） | 从上下文推断；脚本可从 YAML 文件名自动推断，通常不必传 |
| `evidence_dir` | Evidence输出目录 | 默认 `.opencode/harness/evidence` |
| `service_log_dir` | 服务进程日志输出目录 | **必须从 .cache 读取**（详见"服务日志目录"章节） |

### YAML路径确定规则（上下文优先）

Agent **必须**按以下优先级确定 `yaml_spec`，禁止直接硬编码路径：

1. **调用方显式指定**：调用方在 prompt 中指定了 YAML 路径 → 直接使用，最高优先级
2. **从 module 上下文推断**：已获得 module 名时 → 搜索 `e2e/*.yaml`，唯一匹配则使用
3. **自动发现**：以上均无匹配时 → 扫描 `.opencode/harness/e2e/*.yaml`，存在多个则循环执行

### 服务日志目录（service_log_dir）

> 路径由本 skill 获取并传给脚本；日志内容的消费由下游 skill 负责。

**Agent 行为契约**：

```yaml
# ─── 首次执行（无 .cache 文件）───
1. 读 .opencode/skills/harness-dev/harness-run-e2e-test/.cache/service-log-dir.toml
2. 文件不存在 → 询问用户日志目录位置（用 question 工具）
3. 用户回答路径（**无默认值**） → 写入 .cache 文件，内容为：
       service_log_dir = "/用户提供的/绝对/路径"
4. 调用脚本时传入 -ServiceLogDir / -s 参数

# ─── 后续执行（有 .cache 文件）───
1. 读 .cache 文件 → 解析 TOML（正则匹配 service_log_dir = "..." 即可）
2. 直接传给脚本，不再询问用户
```

**TOML 文件格式**（极简，只有一行）：

```toml
service_log_dir = "C:/wanglong/temp/kafka-rust/logs"
```

**禁止给默认值**：日志目录必须由用户显式提供，因为日志可能敏感（含 Kafka 连接信息、token 等），默认路径会污染用户工作区。

---

## 调用方式

### 环境检测

本项目运行在 Windows 环境，默认使用 PowerShell（`run-e2e-test.ps1`）。若检测到 WSL（`$env:WSL_DISTRO_NAME` 存在或 shell 类型为 bash），则改用 `run-e2e-test.sh`。

### 脚本参数

所有参数均来自上方"输入参数"表，脚本参数表只负责 CLI flag 映射：

| CLI 参数 (PS / Bash) | 对应输入参数 |
|---|---|
| `-YamlSpec` / `-y` | `yaml_spec` |
| `-Module` / `-m` | `module` |
| `-EvidenceDir` / `-e` | `evidence_dir` |
| `-ServiceLogDir` / `-s` | `service_log_dir` |
| `-SeqId` / `-n` | `seq_id` |

> 测试场景配置（base_url、skip_ssl_verify、priority、dependencies、readiness、commands 等）均通过 YAML 文件传递，脚本自行解析。`seq_id` 是唯一通过 CLI 参数传递的全局替换变量，脚本在所有 command 字段中执行 `${seq_id}` 替换。

---

## 优先级分类规则

Skill输出每个失败结果的优先级，**阻断判定权归subAgent**。

### 前置条件失败优先级

| 失败类型 | 优先级 | 说明 |
|----------|--------|------|
| `build_failure` | **HIGH** | 无法构建，所有后续测试无效，必须修复 |
| `startup_failure` | **HIGH** | 服务无法启动（仅当 given 包含 startup_command 时触发），必须修复 |
| `dependency_unavailable` | **LOW** | 外部依赖不可用，输出SKIPPED，非代码问题 |

### 场景失败优先级

来源：YAML中每个scenario的`priority`字段（脚本自动读取），REST 和 Commands 模式均适用。

| YAML priority | 含义 | 说明 |
|---------------|------|------|
| **HIGH** | 阻断性失败 | 核心API合约违规（如核心CRUD端点返回错误状态码），必须修复才能通过检视 |
| **LOW**（默认） | 信息性失败 | 非核心功能差异，记录但不阻断检视通过 |

### Evidence 中的 `blocking` 字段

脚本已实现优先级传播，Evidence JSON 中包含：
- `summary.blocking`: `true` 表示存在阻断性问题（HIGH 优先级失败或前置条件失败）
- `summary.high_priority_failures`: HIGH 优先级 FAIL 的数量
- `summary.low_priority_failures`: LOW 优先级 FAIL 的数量

---

## Evidence 消费契约

脚本在 `EvidenceDir` 下生成 `<module>-e2e-result.json`。**完整 JSON 结构的权威在脚本注释**；下表仅列出 Agent 解读和决策所需的字段。

### Agent 必须解读的字段

| 字段路径 | 含义 | Agent 行为 |
|----------|------|-----------|
| `summary.blocking` | 是否存在阻断性问题（HIGH 优先级失败或前置条件失败） | `true` → skip 后续任务，聚焦修复 |
| `summary.overall_result` | 整体结果：`PASS` / `FAIL` / `SKIPPED` | `PASS` → 继续；`FAIL` → 触发修复；`SKIPPED` → 不阻断，记录原因 |
| `summary.high_priority_failures` | HIGH 优先级 FAIL 数量 | > 0 时作为阻断依据 |
| `summary.low_priority_failures` | LOW 优先级 FAIL 数量 | > 0 时记入 non_blocking_issues，不阻断 |
| `prerequisite_check.build_success` | 构建是否成功 | `false` → HIGH 优先级，阻断 |
| `prerequisite_check.startup_success` | 服务启动是否成功 | `false` → HIGH 优先级，阻断 |
| `tests[].name` | 场景名称 | 定位失败场景 |
| `tests[].mode` | 场景模式：`"rest"` / `"commands"` | 区分失败类型（HTTP断言 vs 命令断言） |
| `tests[].result` | 场景结果：`PASS` / `FAIL` / `SKIP` | 筛选失败项 |
| `tests[].priority` | 场景优先级（来自 YAML） | 决定修复紧迫度 |
| `tests[].error_message` | 失败原因（仅 FAIL 时存在） | 构造修复建议 |

---

## 与 subAgent 协作

### subAgent（code-review-agent）消费 E2E 结果

subAgent 在 Phase 2.5 中消费本 Skill 的输出，**阻断判定权归 subAgent**：

1. 读取 `<module>-e2e-result.json` evidence
2. 检查 `summary.blocking` 字段（脚本已计算）
3. **`blocking: true`** → 加入 `blocking_issues`，subAgent 判定阻断检视
4. **`low_priority_failures > 0`** → 加入 `non_blocking_issues`，记录但不阻断
5. **`overall_result = SKIPPED`** → 不阻断，标注跳过原因

### 主 Agent 处理阻断

当 subAgent 报告 HIGH 优先级阻断时：
- 主 Agent 应 skip 其他待执行的 subAgent 任务
- 聚焦修复 HIGH 优先级阻断问题
- 修复后重新触发检视
- LOW 优先级问题可在后续迭代处理

---

## 常见问题

### 环境相关

**Q: Windows和WSL如何切换？**
A: 脚本自动检测环境，无需手动切换。


---

## 禁止事项

1. ❌ 直接在SKILL.md中硬编码测试场景（应在YAML中定义）
2. ❌ 修改脚本代码来适配新测试（应修改YAML）
3. ❌ 在Skill中做阻断判定（阻断判定权归subAgent）
4. ❌ 跳过evidence解读（必须读取和解读evidence文件）
5. ❌ 输出不含优先级分类的验证结果

## 强制事项

1. ✅ 环境检测 → 选择对应脚本
2. ✅ 读取YAML → 调用脚本 → 读取evidence
3. ✅ 每个失败结果附带优先级分类（HIGH/LOW）
4. ✅ Skill不做阻断判定（阻断判定由subAgent负责）
5. ✅ 证据文件写入 `.opencode/harness/evidence/`
6. ✅ 直接消费 evidence 中的 `blocking` 字段做阻断决策

---

## 相关 Skill

| Skill | 职责 | 与本 Skill 的关系 |
|-------|------|------------------|
| [`generate-e2e-test-guide`](../generate-e2e-test-guide/SKILL.md) | 生成 YAML 测试规格 | **上游**：定义YAML格式 |
| [`harness-code-review`](../harness-code-review/SKILL.md) | 编码后代码检视 | **下游**：消费本Skill的evidence |
