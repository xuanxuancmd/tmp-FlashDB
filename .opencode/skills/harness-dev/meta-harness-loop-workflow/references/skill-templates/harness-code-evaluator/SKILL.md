# 评估编排 Skill 模板

> 生成 harness-code-evaluator Skill 时，按此模板填充。
> 此 Skill 教 workflow 的 Maker 如何调用 evaluator agent、如何消费报告。
> 修复循环逻辑统一引用 `fixing-loop.md`，本模板不重复。
> `{target_lang}` 影响 prompt 中的构建命令提示。

---

```markdown
---
name: harness-code-evaluator
description: >-
  评估编排技能。调用 code-evaluator-agent 对编码产物进行独立评估，
  驱动主 Agent 根据报告循环修复直到通过。
  触发词：对抗性审查、evaluator、评估、修复验证、审查本次改动。
---

# harness-code-evaluator Skill

## 职责

指导主 Agent 完成评估环节：调用 evaluator agent → 读取报告 → 判定 pass/fail。

- `pass=true` → 评估通过，交回 workflow 推进下一 stage
- `pass=false` → 进入修复循环（**引用 `fixing-loop.md`**，`trigger_stage=evaluating`，`max_rounds=5`）

> 修复循环的完整流程（修前检查、executor 派发、修后检查、重评估、终止处理）由 `fixing-loop.md` 统一定义，本 skill 不重复。

---

## Caller 契约

调用 `code-evaluator-agent` 时，prompt 中**必须包含**：

| 参数 | 必填 | 说明 |
|------|------|------|
| requirement_source | 是 | Plan 文件路径 / 原始需求文本 / 上下文摘要（agent 自动判断类型） |
| worktree_path | 可选 | git worktree 目录路径（单 Plan 模式为空=项目根目录） |

### 调用前自检

- **Plan 文件场景**：Plan 文件已生成且路径正确
- **文本场景**：从上下文提取的原始需求足够清晰
- **通用**：代码已完成 `{build_cmd}`（避免将编译失败带入评估）

---

## 调用方式

```
task(
  subagent_type="code-evaluator-agent",
  description="代码评估",
  prompt="""
    requirement_source: Plan 路径: {plan_path}        (Plan 文件时)
    requirement_source: {用户原始需求/问题描述}      (无 Plan 时)
    worktree_path: {worktree_path}                    (多 Plan 时必填)
  """
)
```

**等待返回报告路径**后继续。

---

## 报告消费指南

### 入口：`{evidence_dir}/code-evaluator-agent-review.json`

| 文件 | 用途 |
|------|------|
| `code-evaluator-agent-review.json` | 结构化数据，程序化消费 |
| `code-evaluator-agent-review.md` | 人可读详情 |

消费步骤：

1. **Read JSON 报告** → 提取 `overall_result.pass` 和 `blocking_issues[]`
2. `pass=true` → 评估通过 ✅
3. `pass=false` → 遍历 `blocking_issues[]`，对每个 issue：
   - 读取 `location` 定位代码位置
   - 读取 `expected` 和 `actual` 理解差距
   - 读取 `requirement_ref` 关联回原始需求
   - 按严重度分类处理

### blocking_issues 严重度

| 严重度 | 类型 | 处理 |
|--------|------|------|
| **HIGH** | missing / incomplete / wrong / unresolved / regression | 传给 executor(mode=fix) 修复 |
| **MEDIUM** | 任意 | 主 Agent 评估后决定：可接受则跳过，否则传给 executor 修复 |

> 完整的 issues 分类处理表和修复循环流程见 `fixing-loop.md`。

---

## 修复后验证（executor 内部完成）

executor(mode=fix) 在修复过程中会自行运行 build+test 自检。主 Agent 在重新拉起 evaluator 前，无需自行运行构建命令（executor 返回 status=completed 即表示 build+test 已通过）。

> executor 内部的构建/测试命令由 executor agent 定义中的 `{build_cmd}` / `{test_cmd}` 决定，与本 skill 无关。

---

## 禁止事项

1. ❌ pass=false 时跳过修复直接交付
2. ❌ 忽略 HIGH blocking_issues 继续其他工作
3. ❌ 在未读取 JSON 报告的情况下决定修复方案
4. ❌ 主 Agent 自行修复代码而不拉起 executor(mode=fix)（修复必须委托 executor sub-agent）
5. ❌ 评估-修复循环中除达到 5 次上限的 `status=blocked` 终态外，禁止 `question()` 向用户提问
6. ❌ 评估未通过时禁止 `question()` — 必须自动进入修复流程
7. ❌ 修复后禁止 `question()` 确认 — 必须自动重新调用 evaluator 验证

## 强制事项

1. ✅ 优先处理 HIGH 问题，MEDIUM 可评估后跳过
2. ✅ 修复后必须重新调用 evaluator 验证
3. ✅ 每次循环前完整读取 JSON 报告，不凭记忆
4. ✅ 5 次失败后设 `status=blocked`：阻断问题、非阻断问题、报告路径写入 evidence（终止处理见 `fixing-loop.md`）
```

---

## 模板变量说明

| 变量 | 来源 | 示例值 |
|------|------|--------|
| `{skill_name}` | 用户输入或自动 | `harness-dev-workflow` |
| `{target_lang}` | Step 2 | `rust` |
| `{build_cmd}` | Step 2 | `cargo check` |
| `{test_cmd}` | Step 2 | `cargo test` |
| `{evidence_dir}` | 用户输入或默认 | `.opencode/harness/evidence` |
| `{package}` | 运行时参数 | `connect-runtime` |
| `{plan_path}` | 运行时参数 | `.sisyphus/plans/runtime-plan.md` |

## 生成规则

1. 写入 `.opencode/skills/harness-dev/harness-code-evaluator/SKILL.md`（OpenCode）或对应 Claude Code 路径
2. 模板 `task()` 调用中的 `subagent_type` 必须固定为 `"code-evaluator-agent"`
3. 编译/测试命令表从 Step 2 推断
4. **不重复修复循环逻辑**——修复循环由 `fixing-loop.md` 统一定义，本 skill 只负责评估调用 + 报告消费 + pass/fail 判定
5. 总行数建议 60-100 行（比旧版精简，因修复循环逻辑移至 fixing-loop.md）
