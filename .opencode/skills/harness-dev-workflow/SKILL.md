---
name: harness-dev-workflow
description: >-
  自主编码闭环 Skill。按 Plan 编排编码→评估→检视→修复→完成,支持单 Plan / 多 Plan(并发 sub-agent)两种模式。
---

# harness-dev-workflow

## 职责

Plan→编码→质量门禁→修复→完成的自主闭环。不执行需求分析或 Plan 生成(Plan 是输入)。

## 全自动化约束（核心原则）

本 workflow 全自动执行,**唯一暂停例外是显式 `question()` 调用**。

- ✅ **允许暂停**:workflow 内显式 `question()`(当前 2 处:Sub-agent BLOCKED 决策、状态恢复 blocked/completed 决策)
- ❌ **禁止**:Agent 自主暂停(如"等待用户回应"、"上报后停止"、非 `question()` 的文字询问)
- ❌ **禁止**:隐式请求介入(如"向用户报告后等待"、"请求用户决策"等非 `question()` 表述)
- ✅ **替代方式**:需要人工介入但不满足 `question()` 场景时,用异步通道(写报告文件 + 继续执行/终止,不等待)

> sub-skill 内部的 `question()`(如 CatA 审批、evaluator 5 次失败)由 sub-skill 自行定义,workflow 不重复硬编码。

## 输入

### 必选(二选一)

| 参数 | 说明 | 示例 |
|------|------|------|
| `plan_list` | Plan 文件路径列表 | `[".sisyphus/plans/fdb-kvdb-plan.md"]` |
| `requirement_text` | 一句话需求(当 plan_list 为空时必填) | "翻译 fdb_kvdb.c 模块" |

### 可选

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `module_name` | 模块名(用于 state 命名) | 从 plan 文件名或 requirement 推断,项目级默认 `fdb` |

## 预检(Pre-flight)

每次启动时执行,任一失败 → BLOCKED:

1. **State 续传**:Read `.opencode/harness/state/{module}-workflow-state.json`;按 status 分支处理(详见 `references/state-schema.md` 状态恢复段)
2. **Plan 可用**:`plan_list` 为空 → 从 `requirement_text` 构建 plan_list;也为空 → BLOCKED

## 执行模式判定

- `plan_list.length == 1` → 单 Plan 执行,加载 `references/workflow-single-plan.md`
- `plan_list.length > 1` → 多 Plan 执行(强制并行 sub-agent,每个 Plan 独立 code-executor agent),加载 `references/workflow-multi-plan.md`

## 执行上下文

@./references/workflow-single-plan.md
@./references/workflow-multi-plan.md
@./references/fixing-loop.md
@./references/phase-state-machine.md
@./references/state-schema.md

## 禁止事项

1. ❌ 修改 Plan / 测试文件来"通过"(含删除失败测试)
2. ❌ 不读错误输出就重试,或用相同代码重试
3. ❌ Task 级执行 evaluating/incremental_reviewing/full_reviewing/fixing(只在 phase 层级)
4. ❌ AI 自评替代确定性门禁(必须执行命令或委托 sub-agent)
5. ❌ 漏刷新 state.json(每个时机必须全部执行)
6. ❌ 多 Plan 主 Agent 直接编码(必须用 `subagent_type="code-executor-agent"` + `background=true` 并行派发)
7. ❌ 使用 oh-my-openagent plugin 的 `run_in_background` / `background_output` / `background_cancel` 工具(OpenCode 原生 `task(background=true)` 已内置完成通知机制)
8. ❌ 有文件修改冲突的 Plan 并发派发(主 Agent 必须分析每个 Plan 的 `files_modified`，将修改相同文件的 Plan 串行化)
9. ❌ Sub-agent 写主 Agent 的 state.json(只写各自 plan 对应的 state_path)
10. ❌ 未确认各 plan state.json 的 status="completed" 就进入 Phase 3
11. ❌ 修改 `truth_source_path` 中的 Plan 路径数组
12. ❌ Agent 自主暂停(非显式 `question()` 的任何暂停行为)
13. ❌ 隐式请求介入(用文字描述"等待用户"等代替 `question()` 调用)
