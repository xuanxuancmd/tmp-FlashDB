# 多 Plan 执行 Workflow（强制并行 code-executor sub-agent）

`plan_list.length > 1` 时，强制为每个 Plan 派一个独立的 `code-executor` sub-agent（`mode: subagent`，全新上下文）。不依赖 oh-my-opencode plugin 的 `category` 路由，使用 OpenCode 原生 `task` 工具的 `subagent_type` + `background=true` 参数（Go 二进制内置能力，无需 plugin）。

## 总流程

```
[主 Agent Step 0 编排清单] 
  → Phase 1: 并行派发 code-executor sub-agent（每个 sub-agent 独立上下文，background=true）
  → Phase 2: 等待所有 sub-agent 完成，汇总结果
  → Phase 3: 主 Agent 全局检视
  → [主 Agent completing + Self-Check]
```

## 主 Agent Step 0: 编排清单初始化（强制执行，不可跳过）

主 Agent 在启动任何 sub-agent 前，**必须**调用 TodoWrite 创建编排清单。跳过此步骤视为流程违规。

### 主 Agent todo 模板

```
1. ☐ Phase 1: 并行派发每个 Plan 的 code-executor sub-agent（subagent_type + background=true）
2. ☐ Phase 1: 等待所有 sub-agent 完成（读各 plan state）
3. ☐ Phase 2: 汇总各 plan 结果（检查 status，处理 blocked）
4. ☐ Phase 3 evaluating: 调用 harness-code-evaluator（跨 Plan 整体评估）
5. ☐ Phase 3 full_reviewing: 加载检视技能执行（占位符为空则跳过）
6. ☐ completing: 刷新主 state status=completed + Self-Check
```

## Phase 1: 并行编码 + 增量检视(每个 sub-agent 独立上下文)

主 Agent 为每个 Plan 并行派发独立的 `code-executor` sub-agent。**不依赖 `category` 路由**（oh-my-opencode plugin），使用 OpenCode 原生 `task` 工具的 `subagent_type` + `background=true` 参数（Go 二进制内置能力，无需 plugin）。每个 sub-agent 拥有全新上下文（OpenCode 天然 session 隔离），互不干扰。

```
task(subagent_type="code-executor-agent", background=true,
     description="Execute plan {plan_name}",
     prompt="""
       plan_path: {plan_path}
       module_name: {module}
       state_path: .opencode/harness/state/{module}-{plan}-state.json
     """)
```

### Sub-agent 子流程（每个 code-executor 独立执行编码，由 agent .md 定义）

**executor 只负责编码 + 测试 + 自检，不执行任何 evaluating / reviewing。** 每个 sub-agent 在自己的独立 session 中执行：

```
[Step 0 清单初始化] → coding → completing (status=completed, build+test 全部通过)
```

> **职责边界**：
> - ✅ 实现 Plan 中的每个 task
> - ✅ 为每个 task 编写/维护测试（TDD 或后置）
> - ✅ 每个 task 完成后执行 `cargo check` + `cargo test` 自检
> - ✅ 编码过程中的 deviation 自动修复（每个 task 最多 3 次）
> - ✅ 创建 Summary（完成状态 + 偏差记录）
> - ❌ 不调用 harness-code-evaluator 或任何 review agent
> - ❌ 不执行 Fixing 子流程（Fixing 由 workflow 层统一负责）
> - ❌ 不刷新主 Agent 的 state.json

#### Sub-agent Step 0: 流程清单初始化（强制执行，不可跳过）

每个 sub-agent 在编码前，**必须**调用 TodoWrite 创建流程清单。跳过此步骤视为流程违规。

**sub-agent todo 模板**:

```
1. ☐ 编码：实现当前 Plan 的所有 task（每 task 后自检 build+test）
2. ☐ 完成（completing）：刷新 plan state status=completed + Self-Check
```

#### Sub-agent HARD GATE — coding 完成验证

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| coding → completing | ① Plan 中所有 task 已实现 ② 所有 task 的 `cargo check` + `cargo test` 均已通过 ③ plan state.json 的 `tasks_remaining` 为空数组 ④ TodoWrite "编码"项已 completed ⑤ 每个 task 的 commit 已记录 | 任一未满足 → BLOCKED，返回编码阶段 |

#### Sub-agent completing Self-Check

在刷新 plan state status=completed 前，**回溯验证**编码阶段已实际执行：

1. Read plan state.json → `tasks_completed` 包含 Plan 中所有 task_id
2. `tasks_remaining` 为空数组
3. 最近一次 `cargo check` / `cargo test` 执行结果均 pass
4. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到编码阶段重新执行，不标记 completed。**

#### Sub-agent success_criteria

- [ ] 当前 Plan 中所有 task 已编码实现并提交
- [ ] 所有 task 的 `cargo check` + `cargo test` 均已通过
- [ ] plan state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（tasks 全部完成，无遗留阻塞）

## Phase 2: 等待 + 状态汇总（并行 sub-agent 全部完成后）

所有 `background=true` 派发的 sub-agent 完成后，主 Agent 等待原生 `<task id="..." state="completed">` XML 通知（由 OpenCode 自动注入会话），逐个读 plan state 文件：

- 无需 `background_output`（原生机制自动返回 sub-agent 完整输出）
- Read 各 plan 的 `state_path` → 确认 `status="completed"`
- 任一 plan status="blocked" → 主 Agent 汇总 blocked-reports，进入全局 fixing 或直接终止

## Phase 3: 全局检视(主 Agent 直接执行)

主 Agent 在主干执行:

```
evaluating → full_reviewing → completing
```

- evaluating: `harness-code-evaluator` skill(跨 Plan 整体评估)
- full_reviewing(占位符为空则跳过):

## 阶段详情

- **编码**:加载以下技能:

  | 技能 | 用途 |
  |------|------|
  | c-translate-to-rust | C→Rust 1:1 翻译规则与决策框架（强制翻译表、禁令表、易错表、复杂场景参考） |

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **incremental_reviewing**(占位符为空则跳过此阶段):

  调用 `code-review-agent`（增量模式，传入本次 Plan 变更的源码文件路径列表）：

  ```
  task(
    subagent_type="code-review-agent",
    description="增量代码检视",
    prompt="review_path: {本次 Plan 变更的源码文件路径，多个用逗号分隔}
  module: {module_name}"
  )
  ```

  > `code-review-agent` 通过 `review_path` 格式自动判断检视类型：多个单文件路径 → 增量检视；目录路径 → 全量检视。

- **full_reviewing**(占位符为空则跳过此阶段):加载以下技能:

  | 技能 / Agent | 用途 |
  |--------------|------|
  | code-review-agent | 通用代码检视（全量模式，传入模块 src/ 目录路径触发） |
  | parity-verifier | 跨语言行为等价性验证（对照 C 源码与 Rust 译文检查行为一致性） |

- evaluating、incremental_reviewing、full_reviewing 任一阶段失败 → Fixing(见 `fixing-loop.md`)。

## 主 Agent HARD GATE — 全局阶段切换验证

主 Agent 在 Phase 3 全局检视的阶段切换处必须通过验证，未通过则 BLOCKED。**这不是建议，未通过门禁的阶段切换是非法的。**

| 切换点 | 验证项 | 失败处理 |
|--------|--------|---------|
| Phase 2 汇总完成 → Phase 3 evaluating | ① 所有 plan_status="completed" ② 主 state 已刷新 phase="evaluating" ③ TodoWrite "Phase 2 汇总"项已 completed | 任一未满足 → BLOCKED |
| Phase 3 evaluating → full_reviewing | ① evaluating 报告已生成 ② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "Phase 3 evaluating"项已 completed | 任一未满足 → BLOCKED，返回 evaluating |
| Phase 3 full_reviewing → completing | ① full_reviewing 报告已生成（或占位符为空已记录跳过）② 报告无 HIGH blocking issues（或已进入 Fixing 修复）③ TodoWrite "Phase 3 full_reviewing"项已 completed | 任一未满足 → BLOCKED，返回 full_reviewing |

## 主 Agent completing Self-Check

在刷新主 state status=completed 前，**回溯验证**所有阶段已实际执行：

1. Read 主 state.json → 确认所有 plan_status="completed"
2. 确认 Phase 3 evaluating 报告存在
3. 确认 Phase 3 full_reviewing 报告存在（或占位符为空已记录跳过）
4. TodoWrite 所有项已标记 completed

**任一缺失 → 回退到缺失阶段重新执行，不标记 completed。**

## 主 Agent success_criteria

- [ ] 所有 Plan 的 sub-agent 已完成（plan_status 全部 completed）
- [ ] 所有 plan state.json 均已读回，status="completed"
- [ ] Phase 3 evaluating 阶段已执行（harness-code-evaluator 已调用）
- [ ] Phase 3 full_reviewing 阶段已执行（或占位符为空已跳过）
- [ ] 主 state.json 已刷新为 status="completed"
- [ ] TodoWrite 所有项已标记 completed
- [ ] Self-Check 通过（所有阶段报告存在）
