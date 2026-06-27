---
name: harness-code-review
description: Harness编码完成后的检视技能。指导主Agent触发检视SubAgent、获取检视报告、循环修复问题直到通过或达到最大重试次数。
---

# Harness 检视 Skill

## 职责

指导主Agent完成通用检视-修复循环：触发检视 → 读取报告 → 修复问题 → 重新检视（最多3次）。

> **翻译项目请使用 `harness-translate-code-review` skill**，它会并行编排通用检视 + 翻译忠实度检视两个Agent。

## Evidence消费原则

### 原则1：summary.json是决策入口

先Read `<module>-review-summary.json`。
- `overall_result.pass=true` → 任务完成 ✅
- `overall_result.pass=false` → 进入修复循环，从 `blocking_issues[]` 提取阻断性问题

### 原则2：按需读取evidence文件

summary.json中每个检查项含evidence_file路径。当检查项pass=false时，Read对应evidence文件定位具体问题：

| 检查项失败 | 读取文件 | 处理方式 |
|-----------|---------|---------|
| placeholder_detection | placeholder.json | 直接修复（补全规范声明 / 实现占位符） |
| build_check | build-result.txt | 定位编译错误，修复 |
| test_check | test-result.txt | 定位失败用例，修复 |
| AI检查 | ai-check-test.json / ai-diff.json | 定位具体问题，修复 |

### 原则3：report.md作为补充阅读

Read `<module>-review-report.md` 获取详细分析、修复建议（人阅读视角）

## 检视触发

```
task(
  subagent_type="code-review-agent",
  description="检视模块",
  prompt="检视路径: {review_path}"
)
```

prompt必须包含 `review_path`。SubAgent完成后返回报告路径，主Agent用Read工具读取。

## 循环修复流程

1. 启动检视SubAgent → 等待返回路径
2. Read summary.json（原则1）→ 判断pass/fail
3. pass=true → 任务完成，交付用户
4. pass=false → 按需Read evidence文件（原则2）→ Read report.md（原则3）→ 定位问题并修复
5. 修复后重新检视（retry += 1）
6. 达到3次仍未通过 → 向用户报告失败，请求介入

## 禁止事项

1. ❌ 直接执行校验脚本（应由SubAgent执行）
2. ❌ 忽略HIGH blocking_issues继续其他工作
3. ❌ 超3次循环后继续自动重试
4. ❌ 未通过检视时跳过修复直接交付

## 强制事项

1. ✅ 区分HIGH/LOW优先级，HIGH阻断时skip其他subAgent
2. ✅ 遵循3次循环上限，达到后请求用户介入
3. ✅ 所有blocking_issues必须修复后才能pass

## 检视流程图

```
┌───────────────────────────────────────────────────────────────┐
│   检视-修复循环 (max_retries=3)                                │
├───────────────────────────────────────────────────────────────┤
│                                                               │
│  current_retry = 0                                            │
│                                                               │
│  ┌───────────────────────────────────────────────────────┐   │
│  │                    循环开始                             │   │
│  │                       ↓                               │   │
│  │  current_retry >= 3 ?                                 │   │
│  │      ├─ YES → 报告失败，请求用户介入                   │   │
│  │      └─ NO  → 继续                                    │   │
│  │                     ↓                                 │   │
│  │  current_retry += 1                                   │   │
│  │                     ↓                                 │   │
│  │  task(subagent_type="code-review-agent")             │   │
│  │                     ↓                                 │   │
│  │  检视SubAgent执行校验                                 │   │
│  │                     ↓                                 │   │
│  │  Read(summary.json) → pass?                           │   │
│  │      ├─ YES → 任务完成 ✅                             │   │
│  │      └─ NO  → Read(evidence) → Read(report.md)       │   │
│  │              → 定位问题并修复                          │   │
│  │              ↓                                        │   │
│  │  修复完成 → 返回循环开始                               │   │
│  │                                                        │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

## SubAgent详细定义

检视SubAgent的执行流程、权限、报告格式 → `.opencode/agents/code-review-agent.md`
