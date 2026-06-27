# 评估编排 Skill 模板

> 生成 harness-code-evaluator Skill 时，按此模板填充。
> 此 Skill 教 workflow 的 Maker 如何调用 evaluator agent、如何消费报告、如何控制重试循环。
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

指导主 Agent 完成「评估 → 修复」循环：调用 evaluator agent → 读取报告 → 提取问题 → 修复 → 重新评估（最多 3 次）。

---

## Caller 契约

调用 `code-evaluator-agent` 时，prompt 中**必须包含**：

| 参数 | 必填 | 说明 |
|------|------|------|
| requirement_source | 是 | Plan 文件路径 / 原始需求文本 / 上下文摘要（agent 自动判断类型） |

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
  prompt="requirement_source: Plan 路径: {plan_path}        (Plan 文件时)
          requirement_source: {用户原始需求/问题描述}      (无 Plan 时)",
  run_in_background=false,
  load_skills=[]
)
```

**等待返回报告路径**后继续。

---

## 修复循环流程

```
current_retry = 0

┌─ 循环开始 ─────────────────────────────────────────────┐
│                                                         │
│  current_retry >= 5 ?                                   │
│      ├─ YES → 停止重试，设 status=blocked（见"终止处理"） │
│      └─ NO  → current_retry += 1                        │
│                                                         │
│  task(subagent_type="{skill_name}-evaluator")           │
│      ↓                                                  │
│  等待返回报告路径                                        │
│      ↓                                                  │
│  Read({evidence_dir}/{skill_name}-evaluator-review.json)│
│      → overall_result.pass?                             │
│      ├─ YES → 审查通过 ✅ 退出循环                       │
│      └─ NO  → Read(blocking_issues[])                   │
│              ↓                                          │
│         对每个 blocking_issue 分类处理                    │
│              ↓                                          │
│         修复完成 → 返回循环开始                           │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### blocking_issues 分类处理

| 严重度 | 类型 | 处理方式 |
|--------|------|---------|
| **HIGH** | missing | 立即补充实现 |
| **HIGH** | incomplete | 立即补全逻辑 |
| **HIGH** | wrong | 立即重写，对照原始需求 |
| **HIGH** | unresolved | 分析根因，重新修复 |
| **HIGH** | regression | 立即回退或修复回归 |
| **MEDIUM** | 任意 | 评估后决定：可接受则跳过，否则修复 |

**优先级规则**：HIGH 阻断性优先处理，修复期间跳过非阻塞工作。

### 终止处理

5 次循环仍未通过时，设 `status=blocked`，不 question 用户：

1. 剩余 HIGH 阻断性问题列表（含位置和描述）写入 evidence 报告
2. 剩余 MEDIUM 非阻断性问题汇总写入 evidence 报告
3. 最后一次审查的报告路径记录在 state.json 的 `blocked_reason`
4. state.json 刷新为 `status=blocked`，由终态机制触发人工介入

---

## 报告消费指南

### 入口：`{evidence_dir}/{skill_name}-evaluator-review.json`

| 文件 | 用途 |
|------|------|
| `{skill_name}-evaluator-review.json` | 结构化数据，程序化消费 |
| `{skill_name}-evaluator-review.md` | 人可读详情 |

消费步骤：

1. **Read JSON 报告** → 提取 `overall_result.pass` 和 `blocking_issues[]`
2. `pass=true` → 审查通过
3. `pass=false` → 遍历 `blocking_issues[]`，对每个 issue：
   - 读取 `location` 定位代码位置
   - 读取 `expected` 和 `actual` 理解差距
   - 读取 `requirement_ref` 关联回原始需求
   - 按严重度分类处理

---

## 修复后验证命令（语言感知）

修复后、重新调用 evaluator 前，先本地运行确认无编译/测试回归：

| target_lang | 编译验证 | 测试验证 |
|-------------|---------|---------|
| rust | `{build_cmd} [-p {package}]` | `{test_cmd} [-p {package}]` |
| nodejs | `{build_cmd}` | `{test_cmd}` |
| python | `{build_cmd}` | `{test_cmd}` |
| go | `{build_cmd}` | `{test_cmd}` |
| java | `{build_cmd}` | `{test_cmd}` |

> 编译/测试未通过前不要重新调用 evaluator（避免将编译失败带入评估）。

---

## 禁止事项

1. ❌ pass=false 时跳过修复直接交付
2. ❌ 超过 5 次循环后继续自动重试
3. ❌ 忽略 HIGH blocking_issues 继续其他工作
4. ❌ 在未读取 JSON 报告的情况下决定修复方案
5. ❌ 自行修复而不调用 evaluator 重新验证
6. ❌ 评估-修复循环中除达到 5 次上限的 `status=blocked` 终态外,禁止 `question()` 向用户提问
7. ❌ 评估未通过时禁止 `question()` — 必须自动进入修复流程
8. ❌ 修复后禁止 `question()` 确认 — 必须自动重新调用 evaluator 验证

## 强制事项

1. ✅ 优先处理 HIGH 问题，MEDIUM 可评估后跳过
2. ✅ 遵循 5 次循环上限，达到后设 `status=blocked`
3. ✅ 修复后必须重新调用 evaluator 验证
4. ✅ 每次循环前完整读取 JSON 报告，不凭记忆
5. ✅ 5 次失败后设 `status=blocked`：阻断问题、非阻断问题、报告路径写入 evidence
```

---

## 模板变量说明

| 变量 | 来源 | 示例值 |
|------|------|--------|
| `{skill_name}` | 用户输入或自动 | `harness-dev-workflow` |
| `{target_lang}` | Step 0.5 | `rust` |
| `{build_cmd}` | Step 0.5 | `cargo check` |
| `{test_cmd}` | Step 0.5 | `cargo test` |
| `{evidence_dir}` | 用户输入或默认 | `.opencode/harness/evidence` |
| `{package}` | 运行时参数 | `connect-runtime` |
| `{plan_path}` | 运行时参数 | `.sisyphus/plans/runtime-plan.md` |

## 生成规则

1. 写入 `.opencode/skills/{skill_name}-evaluator-review/SKILL.md`（OpenCode）或对应 Claude Code 路径
2. 模板 `task()` 调用中的 `subagent_type` 必须与 Step 5.5a 生成的 agent 名称一致
3. 编译/测试命令表从 Step 0.5 推断
4. 总行数建议 80-120 行
