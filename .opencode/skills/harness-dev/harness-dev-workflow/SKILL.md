---
name: harness-dev-workflow
description: >-
  自主编码闭环 Skill。主 Agent 编排:编码(executor)→检视(code-review-agent)→修复(executor)→评估(code-evaluator-agent)→修复(executor)→完成,支持单 Plan / 多 Plan(多 Plan 默认串行、可选并发)两种模式。
---

# harness-dev-workflow

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

1. **State 续传**:Read `.opencode/harness/state/{module}-workflow-state.json`
   - 不存在 → 全新启动
   - 存在且 status="running" → 按 `references/state-schema.md` 的"状态恢复"段恢复（**多 Plan 场景需核对三层状态**: 主 state + 各 executor state + worktree 一致性）
   - 存在且 status="blocked" → `question()` 上报
   - 存在且 status="completed" → `question("已完成，是否重启?")`
2. **Plan 可用**:`plan_list` 为空 → 从 `requirement_text` 构建 plan_list;也为空 → BLOCKED

## 执行模式判定

- `plan_list.length == 1` → 单 Plan 执行,加载 `references/workflow-single-plan.md`
- `plan_list.length > 1` → 多 Plan 执行(worktree 隔离 + 串行/并发可选),加载 `references/workflow-multi-plan.md`

## 执行上下文

@./references/workflow-single-plan.md
@./references/workflow-multi-plan.md
@./references/fixing-loop.md
@./references/phase-state-machine.md
@./references/state-schema.md

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
