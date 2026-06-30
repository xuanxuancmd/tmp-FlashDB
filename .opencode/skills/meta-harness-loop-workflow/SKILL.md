---
name: meta-harness-loop-workflow
description: >-
  元 Skill：生成项目专用的自主编码闭环 Skill。生成一个自包含的Plan→编码→验证→修复→迭代 闭环 Skill，以及配套的评估 Agent 定义和编排 Skill。
  触发：用户要求"生成编码循环"、"创建自主编码 skill"、"coding loop"、  "自主修复闭环"等。
---

# Harness Loop Template（编码闭环 Skill 生成器）

## 职责

**元 Skill**：不执行任何编码工作。输出为：
1. 一个编码闭环主 Skill（内置 pattern 执行流程 + 验证编排）
2. **编码执行 Agent**（`code-executor-agent`）：独立 subagent，多 Plan 模式下每个 Plan 派一个，拥有完整读写权限，内部烘焙 `coding_skills`（不含 reviewing 技能——executor 只编码）
3. 配套的评估 Agent 定义（`code-evaluator-agent`）+ 评估编排 Skill（语言感知）
4. 必要的控制层 Hook（state 守卫 + 运行日志）

## 输入

用户触发时，收集以下参数（按必选/条件必选/可选分组）：

### 必选参数

| 参数 | 说明 | 示例 |
|------|------|------|
| `skill_name` | 生成的 workflow Skill 名称 | `harness-dev-workflow` |

### 可选参数

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `harness_dir` | harness 工程目录 | `.opencode/harness` |
| `target_lang` | 目标语言（如项目根目录无法推断时使用） | 由 Step 2 自动检测 |

> **说明**：workflow **运行时**需要的参数（如 `plan_source`、`requirement_text`、`module_name`）不属于 meta skill。这些参数由生成的 workflow skill 在运行时接收（详见 `references/skill-templates/workflow/SKILL.md` §输入 段）。

### AI 自动推断的参数（不在输入表中）

以下参数由 meta skill 在 Step 2（语言检测）中**自动推断**，**不由用户输入**：

| 参数 | 推断方式 |
|------|---------|
| `target_lang` | 扫描目标项目根目录的标识文件（Cargo.toml、package.json、go.mod 等） |
| `build_cmd` / `test_cmd` / `lint_cmd` | 根据 `target_lang` 从语言命令表自动匹配 |
| `bash_permissions` | 根据 `target_lang` 从 Step 2 命令表推断（用于 evaluator agent 权限配置） |

## 生成流程

### Step 1: 平台检测

扫描项目根目录，确定目标平台：

1. 检查 `.opencode/` 目录是否存在
2. 检查 `.claude/` 目录是否存在
3. 判定逻辑：
   - 两者都存在 → `detected_platforms = ["opencode", "claudecode"]`
   - 仅 `.opencode/` → `detected_platforms = ["opencode"]`
   - 仅 `.claude/` → `detected_platforms = ["claudecode"]`
   - 都不存在 → 询问用户："初始化哪个平台？opencode / claudecode / 两者都"

将 `detected_platforms` 存入变量，后续步骤使用。

### Step 2: 语言检测

扫描项目根目录，推断 `target_lang`，示例：

| 标识文件 | target_lang | build_cmd | test_cmd |
|---------|-------------|-----------|----------|
| `Cargo.toml` | rust | `cargo check` | `cargo test` |
| `go.mod` | go | `go build ./...` | `go test ./...` |
| `pom.xml` / `build.gradle` | java | `mvn compile` / `gradle build` | `mvn test` / `gradle test` |

**语言命令表**（供 code-evaluator-agent-template 使用）：

```bash
# 根据 target_lang 生成的 bash permission 列表
# Rust:
  "cargo check *": allow
  "cargo build *": allow
  "cargo test *": allow
  "cargo clippy *": allow
```

### Step 3: Harness 目录初始化

自动创建（或补全）`.opencode/harness/` 工程目录结构。该目录用于存放 harness 工程相关的**运行工具（自定义 linter、校验脚本等）与运行结果（state、logs、evidence 等）**。

#### 3.1: 创建子目录

如下目录按需创建，但 README.md **和核心四个子目录**是必备的：

```
.opencode/harness/
├── README.md          # 本目录用途说明（见 0.7b）
├── state/             # [核心] workflow 状态 JSON（{module}-workflow-state.json）— 断点续传
├── logs/              # [核心] 运行日志（{module}-run-log.md，由 loop-governance.ts 自动追加）
├── scripts/           # [核心] 运行工具/脚本（state-guard.py、自定义 linter 等）— hook 调用入口
├── features/          # [可选] BDD 测试场景（*.feature）
└── evidence/          # [可选] 校验输出（*.json / *.md）
```

#### 3.2: 生成 README.md

用 `references/harness/readme-template.md` 模板**自动生成**（若已存在则覆盖，保持一致）。

#### 3.3: AGENTS.md 注入建议

用 `references/harness/agents-md-injection.md` 模板，检查并按需注入（不修改现有内容）。

### Step 4: 扫描项目现有 Skill（按阶段分类）

扫描 `.opencode/skills/` 和 `.opencode/agents/`，将 Skill 按 **workflow 的执行阶段** 分类。

#### 4.1: Skill 阶段扫描表

| 阶段 | 识别关键词（description） | 示例 Skill | 注入占位符 |
|------|------------------------|-----------|-----------|
| **coding** | "编码"、"翻译"、"BDD"、"step definition"、"cucumber"、"实现"、"translate"、"mapping"、"migration" | `java-translate-to-rust`、`harness-bdd-coding` | `{coding_skills}` |
| **evaluating** | "评估"、"对抗性审查"、"evaluator"（仅 Plan 层使用，Task 层无评估环节） | — | 内置skill |
| **incremental_reviewing** | "增量"、"review"、"代码检视"、"code-review"（Skill 支持 mode=incremental，每 Plan 完成后执行） | `harness-code-review` | `{incremental_review_skills}` |
| **full_reviewing** | "全量"、"E2E"、"full review"（Skill 支持 mode=full，所有 Plan 完成后执行） | `harness-translate-code-review`、`harness-run-e2e-test`、`harness-code-review`(full 模式) | `{full_review_skills}` |
| **fixing** | "修复自检"、"fix-self-check"、"修复健康"、"self-check"（修复阶段的思维方式门禁） | `fix-self-check` | `{fixing_skills}` |

**注意**：若 Skill description 中明确支持"增量 + 全量"两种模式（如 `harness-code-review`），它应同时出现在 **incremental_reviewing** 和 **full_reviewing** 两阶段表中。meta 在生成时为两阶段分别烘焙不同的调用方式（如 `mode="incremental"` vs `mode="full"`）。

### Step 5: 展示方案给用户审阅

生成前，向用户展示**核心决策信息**，让用户审阅：

```markdown
## 即将生成: {skill_name}

### 项目基础信息
- **目标语言**: {target_lang}
- **构建命令**: `{build_cmd}` / `{test_cmd}` / `{lint_cmd}`
- **Harness 目录**: `{harness_dir}`

### 阶段 Skill 映射（注入 workflow 模板）

| Workflow 阶段 | Skill / Agent | 备注 |
|--------------|--------------|------|
| coding | {coding_skills} | 编码技能列表（烘焙进 executor agent） |
| evaluating（plan 级） | harness-code-evaluator | 由本流程生成 |
| incremental_reviewing（plan 级） | {incremental_review_skills} | 可空，为空时跳过 |
| full_reviewing（global 级） | {full_review_skills} | 可空，为空时跳过 |
| fixing | {fixing_skills} | 修复自检技能，可空，为空时 Fixing 阶段不加载自检 |

### 即将生成的文件

| 文件 | 用途 |
|------|------|
| `.opencode/skills/{skill_name}/SKILL.md` | 职责+全自动化约束+输入+预检+模式判定+@引用+禁止事项 |
| `.opencode/agents/code-executor-agent.md` | 编码执行 Agent（多 Plan 并发用，`mode: subagent`，烘焙 coding_skills） |
| `.opencode/agents/code-evaluator-agent.md` | 评估 agent 定义（`{target_lang}` 环境配置，只读） |
| `.opencode/skills/harness-code-evaluator/SKILL.md` | 评估编排 skill（≤5 次重试逻辑） |
| `.opencode/plugins/loop-governance.ts` | state 守卫 hook（项目级唯一，已存在则覆盖） |

> **executor vs evaluator Agent**：executor 拥有完整读写权限（`permission.edit: allow`），用于多 Plan 并发编码；evaluator 只读（`permission.edit: deny`），用于跨 Plan 统一评估。参见 `references/agent-templates/code-executor-agent.md`。

[确认生成] [修改参数] [取消]
```

### Step 6: 生成编码闭环 Skill

按 `references/skill-templates/workflow/SKILL.md` 模板生成 workflow Skill 文件：

| 平台 | 写入路径 |
|------|---------|
| OpenCode | `.opencode/skills/{skill_name}/SKILL.md` + `.opencode/skills/{skill_name}/references/*.md` |
| Claude Code | `.claude/skills/{skill_name}/SKILL.md` + `.claude/skills/{skill_name}/references/*.md` |

#### 6.1 参数注入（替换模板中的占位符）

由 meta skill 在生成时**直接替换**模板中的 `{变量}` 占位符：

**A 类：基础参数**（无条件值替换）

| 占位符 | 值来源 | 示例 |
|--------|--------|------|
| `{skill_name}` | 必选参数 | `harness-dev-workflow` |

**B 类：阶段占位符**（从 Step 4 阶段分类结果构建，注入 references/workflow-*.md）

| 占位符 | 值来源 | 注入位置 | 注入形态 |
|--------|--------|---------|---------|
| `{coding_skills}` | Step 4.1 coding 阶段 Skill 列表 | workflow-single-plan.md + workflow-multi-plan.md 编码段 | markdown 表格：`\| 技能 \| 用途 \|` |
| `{incremental_review_skills}` | Step 4.1 incremental_reviewing 阶段 Skill + code-review-agent | workflow-multi-plan.md incremental_reviewing 段 | task() 调用代码块（mode=incremental）|
| `{full_review_skills}` | Step 4.1 full_reviewing 阶段 Skill 列表 | workflow-single-plan.md reviewing 段 + workflow-multi-plan.md full_reviewing 段 | markdown 表格：`\| 技能 \| 用途 \|` |
| `{fixing_skills}` | Step 4.1 fixing 阶段 Skill 列表 | fixing-loop.md（可选，若为空则 Fixing 不加载自检） | markdown 表格 |

> 若某阶段的 Skill 列表为空（如项目没有 E2E Skill），则对应占位符替换为"无（此阶段跳过）"，不删除行。

**替换规则**：
- A 类：meta skill 生成 SKILL.md 时**必须完成替换**
- B 类：meta skill 生成 references/workflow-*.md 时**必须完成替换**
- 生成的文件**不含未替换占位符**

#### 6.2 阶段占位符生成（按 Step 4 分类结果）

使用 Step 4.1 的阶段扫描结果，依次生成各阶段占位符内容：

| 占位符 | 生成来源 | 生成规则 |
|--------|---------|---------|
| `{coding_skills}` | Step 4.1 coding 阶段 Skill 列表 | 每个 Skill 一行：`| <skill_name> | <description 摘要> |` |
| `{incremental_review_skills}` | Step 4.1 incremental_reviewing 阶段 Skill + code-review-agent | task() 调用片段（含 subagent_type + mode=incremental） |
| `{full_review_skills}` | Step 4.1 full_reviewing 阶段 Skill 列表 | 每个 Skill 一行：`| <skill_name> | <description 摘要> |` |
| `{fixing_skills}` | Step 4.1 fixing 阶段 Skill 列表 | 每个 Skill 一行：`| <skill_name> | <description 摘要> |`，为空则 fixing-loop.md 中不加载自检 |

> **跨阶段 Skill 的处理**：同一 Skill 允许出现在 incremental_reviewing 和 full_reviewing，难以判断同意归类到full_reviewing中。

### Step 7: 生成编码执行 Agent + 评估 Agent + 编排 Skill

#### 7.0: Code Executor Agent 定义（多 Plan 并发专用）

用 `references/agent-templates/code-executor-agent.md` 模板生成编码执行 Agent。

| 平台 | 写入路径 |
|------|---------|
| OpenCode | `.opencode/agents/code-executor-agent.md` |
| Claude Code | `.claude/agents/code-executor-agent.md` |

关键变量填充：

| 占位符 | 来源 | 说明 |
|--------|------|------|
| `{skill_name}` | 必选参数 | Agent 文件名前缀 |
| `{project_name}` | 项目名 | Agent description |
| `{build_cmd}` / `{test_cmd}` | Step 2 | Agent 构建/测试命令（写入 agent 文本，供其调用） |
| `{bash_permissions}` | Step 2 语言命令表 | Agent bash permission 块 |
| `{coding_skills}` | Step 4.1 coding 阶段 | 烘焙到 agent `.md`，运行时子 agent 调用 `skill()` 加载 |
| `{incremental_review_skills}` | Step 4.1 incremental_reviewing 阶段 | 注入到 workflow skill 模板的主 Agent Phase 3 段，为空时写入"无（此阶段跳过）" |

**覆盖策略**：若 `.opencode/agents/code-executor-agent.md` 已存在，直接覆盖写入（保持模板与生成产物始终一致）。


#### 7.1: Evaluator Agent 定义

用 `references/agent-templates/code-evaluator-agent.md` 模板生成：

| 平台 | 写入路径 |
|------|---------|
| OpenCode | `.opencode/agents/code-evaluator-agent.md` |
| Claude Code | `.claude/agents/code-evaluator-agent.md` |

关键变量填充（来自 Step 2 语言检测）：
- `{target_lang}` → agent 的 bash permission 表
- `{build_cmd}` / `{test_cmd}` → agent 的构建/测试命令
- `{evidence_dir}` → agent 的报告输出路径

**覆盖策略**：若 `.opencode/agents/code-evaluator-agent.md` 已存在，直接覆盖写入（保持模板与生成产物始终一致）。

#### 7.2: 评估编排 Skill

用 `references/skill-templates/harness-code-evaluator/SKILL.md` 模板生成：

| 平台 | 写入路径 |
|------|---------|
| OpenCode | `.opencode/skills/harness-code-evaluator/SKILL.md` |
| Claude Code | `.claude/skills/harness-code-evaluator/SKILL.md` |

此 Skill 教 workflow 主 Skill 如何调用 evaluator agent、如何消费报告、如何控制重试循环。

### Step 8: 生成控制层 Hook 与状态守卫脚本

生成项目级唯一的流程治理 hook + 跨平台状态守卫脚本。**不为每个 workflow 生成独立 hook**（避免多 hook 同时监听 state 写入导致重复日志/双重阻断）。

#### 8.1: loop-governance.ts（项目级唯一）

| 平台 | 写入路径 |
|------|---------|
| OpenCode | `.opencode/plugins/loop-governance.ts`（**文件名固定**，不加 skill_name 前缀） |
| Claude Code | 按 `references/claudecode/stop-guard-claudecode.py` 生成完成阻断守卫（详见 7d） |

使用模板：`references/opencode/hook-opencode.md`

`harness-validator.ts` 已存在则覆盖（保持一致）

#### 8.2: state-guard.py 部署（所有平台通用）

**前置条件**：Step 3.1 已创建 `{.opencode|.claude}/harness/scripts/` 核心子目录（若缺失则报错，**不要在此步骤内自行创建**，应回头修复 Step 3）。

**部署动作**：将 `references/scripts/state-guard-template.py` **复制**（不是重命名，保留模板）到 `{.opencode|.claude}//harness/scripts/state-guard.py`。

若 `{.opencode|.claude}//harness/scripts/state-guard.py` 已存在，直接覆盖写入（保持与模板一致）。

#### 8.3: Claude Code 自动注册阻断守卫（仅 Claude Code 平台）

若 `detected_platforms` 包含 `"claudecode"`，执行以下两步：

1. 用 `references/claudecode/stop-guard-claudecode.py` 模板生成 `.claude/hooks/stop-guard.py`
2. 用 `references/claudecode/settings-stop-hook.md` 模板合并 hook 配置到 `.claude/settings.json`

**该 hook 的独有价值**：当 AI 声称完成但 state 显示未完成时，**强制阻止 Claude 结束响应**。OpenCode 无此能力，只能靠 Skill 文字约束。


### Step 10: Plugin 自动注册（仅 OpenCode）

读取 `.opencode/opencode.json` 的 `plugin` 数组，**自动补全**缺失的 plugin 条目：

| 需注册的 Plugin | 检查条件 | 注册值 |
|-----------------|---------|--------|
| `loop-governance.ts` | `plugin` 数组不含 `.opencode/plugins/loop-governance.ts` | `.opencode/plugins/loop-governance.ts` |
| `harness-validator.ts`（若文件存在于 `.opencode/plugins/`） | `plugin` 数组不含 `harness-validator.ts` | `harness-validator.ts` |

**自动注册规则**：
1. 缺失 → **直接写入** `opencode.json`（追加到 `plugin` 数组末尾，不删除已有条目）
2. 已注册 → 跳过
3. `opencode.json` 不存在 → 创建最小配置 `{"plugin": [...]}`
4. 注册完成后输出提示：用户需**重启 OpenCode** 以加载新 plugin

## 闭环工程原则（生成的 Skill 必须体现）

生成时嵌入以下 6 条（规则本身直接内化到 skill 文本中，不引用外部文档）：

1. **双层验证** — AI 评估/检视在前，确定性门禁（review skill 内部 build/test）在后
2. **精确反馈** — 门禁输出具体错误信息（行号、断言、退出码）
3. **重试螺旋防护** — 同错误不重试，诊断→换策略→升级→BLOCKED
4. **Maker/Checker 分离** — 实现者和检视者必须是不同 agent/session
5. **Kill Switch** — 最大迭代数 + 同错误不重试 + 仅 `status=blocked` 时 question() 上报 + state 断点
6. **Checkpoint** — 每个 task 构建检查通过后刷新 state.json，支持断点续传

## 禁止事项

1. ❌ 生成的 Skill 不得包含需求分析或 Plan 生成逻辑（Plan 是输入）
2. ❌ 生成的 Skill 不得允许 AI 修改测试文件来"通过"测试
3. ❌ 生成的 Skill 不得用 AI 自评替代确定性门禁
4. ❌ 生成的 Skill 不得省略 Kill Switch
5. ❌ 生成的 evaluator agent 不得有 Edit/Write 源码权限（只读 + 写报告目录）
6. ❌ 生成的 evaluator agent 的 bash permission 不得包含与 target_lang 无关的编译命令