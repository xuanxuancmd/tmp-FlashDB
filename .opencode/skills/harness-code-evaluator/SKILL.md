---
name: harness-code-evaluator
description: >-
  评估编排技能。调用 harness-dev-workflow-evaluator agent 对编码产物进行独立评估，
  驱动主 Agent 根据报告循环修复直到通过。
  触发词：对抗性审查、evaluator、评估、修复验证、审查本次改动。
---

# harness-code-evaluator Skill

## 职责

指导主 Agent 完成「评估 → 修复」循环：调用 evaluator agent → 读取报告 → 提取问题 → 修复 → 重新评估（最多 3 次）。

---

## Caller 契约

调用 `harness-dev-workflow-evaluator` 时，prompt 中**必须包含**：

| 参数 | 必填 | 说明 |
|------|------|------|
| requirement_source | 是 | Plan 文件路径 / 原始需求文本 / 上下文摘要（agent 自动判断类型） |

### 调用前自检

- **Plan 文件场景**：Plan 文件已生成且路径正确
- **文本场景**：从上下文提取的原始需求足够清晰
- **通用**：代码已完成 `cargo check`（避免将编译失败带入评估）

---

## 调用方式

```
task(
  subagent_type="harness-dev-workflow-evaluator",
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
│  current_retry >= 3 ?                                   │
│      ├─ YES → 停止重试，上报用户（见"终止处理"）          │
│      └─ NO  → current_retry += 1                        │
│                                                         │
│  task(subagent_type="harness-dev-workflow-evaluator")   │
│      ↓                                                  │
│  等待返回报告路径                                        │
│      ↓                                                  │
│  Read(.opencode/harness/evidence/harness-dev-workflow-evaluator-review.json) │
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

3 次循环仍未通过时，向用户报告：

1. 剩余 HIGH 阻断性问题列表（含位置和描述）
2. 剩余 MEDIUM 非阻断性问题汇总
3. 最后一次审查的报告路径
4. 请求用户介入决策

---

## 报告消费指南

### 入口：`.opencode/harness/evidence/harness-dev-workflow-evaluator-review.json`

| 文件 | 用途 |
|------|------|
| `harness-dev-workflow-evaluator-review.json` | 结构化数据，程序化消费 |
| `harness-dev-workflow-evaluator-review.md` | 人可读详情 |

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
| rust | `cargo check [-p {package}]` | `cargo test [-p {package}]` |

> 编译/测试未通过前不要重新调用 evaluator（避免将编译失败带入评估）。本项目 Rust 代码位于 `connect-rust/` 子目录，执行 cargo 命令时需 `cd connect-rust` 或使用 `-p connect-{module}` 指定包。

---

## 禁止事项

1. ❌ pass=false 时跳过修复直接交付
2. ❌ 超过 3 次循环后继续自动重试
3. ❌ 忽略 HIGH blocking_issues 继续其他工作
4. ❌ 在未读取 JSON 报告的情况下决定修复方案
5. ❌ 自行修复而不调用 evaluator 重新验证

## 强制事项

1. ✅ 优先处理 HIGH 问题，MEDIUM 可评估后跳过
2. ✅ 遵循 3 次循环上限，达到后请求用户介入
3. ✅ 修复后必须重新调用 evaluator 验证
4. ✅ 每次循环前完整读取 JSON 报告，不凭记忆
5. ✅ 3 次失败后向用户完整报告：阻断问题、非阻断问题、报告路径
