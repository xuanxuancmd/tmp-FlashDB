# Workflow Skill 模板

> 生成自主编码闭环 Skill 时,按此模板填充。`{变量}` 由 meta skill 替换为项目实际值。

---

## 生成的文件结构

```
.opencode/skills/{skill_name}/
├── SKILL.md                              # 薄入口（≤80 行）
└── references/
    ├── workflow-single-plan.md           # 单 Plan 执行流程（主 Agent 编排，无 worktree）
    ├── workflow-multi-plan.md            # 多 Plan 编排（worktree 隔离 + 串行/并发可选）
    ├── fixing-loop.md                    # Fixing 阶段通用权威（主 Agent 拉起 executor mode=fix）
    └── state-schema.md                   # 统一 state.json schema + 名词映射 + 三层一致性 + 断点续传
.opencode/harness/
└── workflow.yaml                         # 工作流定义（stage 顺序 + on_failure 跳转，状态转移规则单一权威）
```

## 模板文件索引

| 模板文件 | 生成目标 | 说明 |
|---------|---------|------|
| 本文件 SKILL.md | `.opencode/skills/{skill_name}/SKILL.md` | 薄入口（职责+原则+输入+预检+模式判定+禁止事项） |
| `workflow.yaml.example` | `.opencode/harness/workflow.yaml` | 工作流定义（stage 顺序 + on_failure，状态转移规则的单一权威） |
| `workflow-single-plan.md` | `references/workflow-single-plan.md` | 单 Plan 执行流程 |
| `workflow-multi-plan.md` | `references/workflow-multi-plan.md` | 多 Plan 编排（串行/并发可选 + merge 行为专节） |
| `fixing-loop.md` | `references/fixing-loop.md` | Fixing 阶段通用权威（循环流程 + issues 分类 + evidence 消费 + 终止处理） |
| `state-schema.md` | `references/state-schema.md` | State 单一权威（Schema + 名词映射 + 刷新时机 + 三层一致性 + 断点续传） |

## 模板变量说明

| 变量 | 来源 | 注入位置 | 示例值 |
|------|------|---------|--------|
| `{skill_name}` | 用户输入 | SKILL.md frontmatter + 标题 | `harness-dev-workflow` |
| `{module_name}` | 运行时参数 | state schema 注释 + workflow.yaml | `connect-runtime` |
| `{coding_skills}` | Step 4.1 coding 阶段 | executor agent 模板（烘焙进 agent 定义） | markdown 表格 |
| `{incremental_reviewing_skill}` | Step 4.1 incremental_reviewing 子类 | workflow.yaml local-stages review 项 | skill 名 |
| `{full_reviewing_skill}` | Step 4.1 full_reviewing 子类 | workflow.yaml global-stages full_review 项 | skill 名 |
| `{fixing_skill}` | Step 4.1 fixing 阶段 | workflow.yaml optional-stages fix 项 | skill 名 |

## 生成规则

1. SKILL.md 薄入口模板段写入 `.opencode/skills/{skill_name}/SKILL.md`（OpenCode）或 `.claude/skills/{skill_name}/SKILL.md`（Claude Code）
2. 4 个 reference 模板文件写入 `.opencode/skills/{skill_name}/references/`（或 `.claude/skills/{skill_name}/references/`）
3. SKILL.md 行数 ≤80 行（薄入口）
4. 各 references 文件行数无硬性限制，但建议单个文件 ≤180 行
5. 占位符替换：meta skill 在 Step 6 生成时**必须完成所有 {变量} 替换**，生成的文件不含未替换占位符
6. 若某阶段 Skill 列表为空（如项目无 E2E Skill），对应占位符替换为"无（此阶段跳过）"，不删除行

---

## Skill模板（SKILL.md）

> 以下内容为生成的 SKILL.md 模板,meta skill 读取此段写入目标文件。

```markdown
---
name: {skill_name}
description: >-
  自主编码闭环 Skill。主 Agent 编排:编码(executor)→检视(code-review-agent)→修复(executor)→评估(code-evaluator-agent)→修复(executor)→完成,支持单 Plan / 多 Plan(多 Plan 默认串行、可选并发)两种模式。
---

# {skill_name}

## 职责

主 Agent 作为**纯编排者**,协调三类 sub-agent 完成闭环:
- **executor**(`code-executor-agent`):编码(mode=coding) / 修复(mode=fix)
- **reviewer**(`code-review-agent`):代码检视,输出结构化报告
- **evaluator**(`code-evaluator-agent`):Plan↔代码一致性评估,输出结构化报告

主 Agent **不直接编码/检视/评估**,只负责:worktree 管理 → 派发 sub-agent → 读结构化摘要 → 决策下一步 → 修复循环控制 → merge → 完成。

## 架构原则

1. **Maker/Checker 分离**:executor(Maker,可写)与 reviewer/evaluator(Checker,只读)是不同 session
2. **worktree 隔离**(多 Plan):每个 Plan 独立 worktree;主 Agent 管理 worktree 生命周期(创建→传递→merge→清理)
3. **结构化通信**:sub-agent 之间不直接通信,通过主 Agent 传递plan或issue文件路径

## 全自动化约束（核心原则）

本 workflow 全自动执行,**唯一暂停例外是显式 `question()` 调用**。

- ✅ **允许暂停**:workflow 内显式 `question()`(当前 2 处:Sub-agent BLOCKED 决策、状态恢复 blocked/completed 决策)
- ❌ **禁止**:Agent 自主暂停(如"等待用户回应"、"上报后停止"、非 `question()` 的文字询问)
- ❌ **禁止**:隐式请求介入(如"向用户报告后等待"、"请求用户决策"等非 `question()` 表述)
- ✅ **替代方式**:需要人工介入但不满足 `question()` 场景时,用异步通道(写报告文件 + 继续执行/终止,不等待)

> sub-skill 内部的 `question()`(如 CatA 审批、evaluator 失败)由 sub-skill 自行定义,workflow 不重复硬编码。

## 输入

### 必选(二选一)

| 参数 | 说明 | 示例 |
|------|------|------|
| `plan_list` | Plan 文件路径列表 | `[".omo/plans/runtime-plan.md"]` |
| `requirement_text` | 一句话需求(当 plan_list 为空时必填) | "实现 offset 管理功能" |

### 可选

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `module_name` | 模块名(用于 state 命名) | 从 plan 文件名或 requirement 推断,项目级默认 `project` |

## 预检(Pre-flight)

每次启动时执行,任一失败 → BLOCKED:

1. **workflow.yaml 可读**:Read `.opencode/harness/workflow.yaml`
   - 不存在 → BLOCKED（状态转移规则未定义，脚本无法工作）
   - 存在 → 解析 local-stages / global-stages / optional-stages，作为后续 stage 推进的权威
2. **State 续传**:Read `.opencode/harness/state/{module}-workflow-state.json`
   - 不存在 → 全新启动
   - 存在且 status="running" → 按 `references/state-schema.md` 的"状态恢复"段恢复（**多 Plan 场景需核对三层状态**: 主 state + 各 executor state + worktree 一致性）
   - 存在且 status="blocked" → `question()` 上报
   - 存在且 status="completed" → `question("已完成，是否重启?")`
3. **Plan 可用**:`plan_list` 为空 → 从 `requirement_text` 构建 plan_list;也为空 → BLOCKED

## 执行模式判定

- `plan_list.length == 1` → 单 Plan 执行,加载 `references/workflow-single-plan.md`
- `plan_list.length > 1` → 多 Plan 执行(worktree 隔离 + 串行/并发可选),加载 `references/workflow-multi-plan.md`

## 禁止事项

1. ❌ 修改 Plan / 测试文件来"通过"(含删除失败测试)
2. ❌ 不读错误输出就重试,或用相同代码重试
3. ❌ 主 Agent 直接编码/检视/评估(必须委托 sub-agent)
4. ❌ Executor 拉起 sub-agent(executor 的 task 权限已 deny,架构硬约束)
5. ❌ AI 自评替代确定性门禁(必须执行命令或委托 sub-agent)
6. ❌ 漏刷新 state.json(每个时机必须全部执行)
7. ❌ 多 Plan 模式不创建 worktree 就派发 executor(git index.lock 会冲突)
8. ❌ Sub-agent 写主 Agent 的 state.json(只写各自 plan 对应的 state_path)
9. ❌ 未确认各 plan state.json 的 status="completed" 就进入检视/评估阶段
10. ❌ 修改 `truth_source_path` 中的 Plan 路径数组
11. ❌ Agent 自主暂停(非显式 `question()` 的任何暂停行为)
12. ❌ worktree 未 merge 就删除(会丢失代码)
```
