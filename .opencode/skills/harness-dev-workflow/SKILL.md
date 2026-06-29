---
name: harness-dev-workflow
description: >-
  自主编码闭环 Skill。按 Plan 编排编码→评估→等价性验证→检视→修复→完成,支持单 Plan / 多 Plan(并发 worktree)两种模式。
---

# harness-dev-workflow

## 职责

Plan→编码→质量门禁→修复→完成的自主闭环。不执行需求分析或 Plan 生成(Plan 是输入)。

## 输入

### 必选(二选一)

| 参数 | 说明 | 示例 |
|------|------|------|
| `plan_list` | Plan 文件路径列表 | `[".sisyphus/plans/runtime-plan.md"]` |
| `requirement_text` | 一句话需求(当 plan_list 为空时必填) | "实现 fdb_tsdb_get() 功能" |

### 可选

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `module_name` | 模块名(用于 state 命名) | 从 plan 文件名或 requirement 推断，项目级默认 `project` |

## 项目阶段映射

**项目语言**: Rust | **构建**: `cargo check` | **测试**: `cargo test` | **Lint**: `cargo clippy`

### coding 阶段技能

| 技能 | 用途 |
|------|------|
| c-translate-to-rust | C 到 Rust 1:1 翻译实战指南（映射速查表、Rust 禁令表、嵌入式场景） |
| harness-bdd-coding | BDD 测试代码实现（Feature 拷贝、Cucumber 测试生成、Evidence 收集） |
| harness-bdd-design | BDD 测试场景设计（Discovery + Formulation，Gherkin 规格生成） |

### evaluating 阶段（Plan 级一致性评估）

调用 `harness-code-evaluator` skill（Plan↔代码一致性评估）。失败 → Fixing。

### parity_verifying 阶段（行为等价性验证,可选)

调用 `parity-verifier` agent(跨语言行为等价性检查:通用行为检查 + 项目领域不变量)。失败 → Fixing,并优先调用 `failure-resolver` agent 进行诊断。

**触发条件**:项目中存在 `.opencode/agents/parity-verifier.md` 文件时启用。该文件不存在则跳过此阶段,直接进入 `incremental_reviewing` 或 `reviewing`。

> 启用条件与具体语言对无关 — 任何代码迁移项目(legacy→modern、跨语言、跨框架)只要提供自己的 `parity-verifier` agent 即可获得该能力。

### incremental_reviewing 阶段技能（每个 Plan/worktree 完成后）

```
task(
  subagent_type="code-review-agent",
  load_skills=["harness-code-review"],
  description="增量检视",
  prompt="检视路径: {review_path}（mode=incremental，单 Plan 完成后执行）"
)
```

### full_reviewing 阶段技能（所有 Plan 合并后）

| 技能 | 用途 |
|------|------|
| harness-code-review (mode=full) | 全量代码检视（编译、占位符检测、BDD 测试） |

### fixing 阶段技能

| 技能 | 用途 |
|------|------|
| fix-self-check | 修复前自检（因果链诊断 + 爆炸半径论证），修复后自检（回归检测 + 健康趋势） |

## 预检(Pre-flight)

每次启动时执行,任一失败 → BLOCKED:

1. **State 续传**:Read `.opencode/harness/state/{module}-workflow-state.json`;按 status 分支处理(详见状态恢复段)
2. **Plan 可用**:`plan_list` 为空 → 从 `requirement_text` 构建 plan_list;也为空 → BLOCKED

## 单 Plan 执行

符合`plan_list.length == 1` 为单plan。

### 流程

```
[编码 task 1..N] → [evaluating] → [parity_verifying] → [reviewing] → [completing]
```
*此处`reviewing`等同于 full_reviewing*

### 详情

- **编码**:加载 `c-translate-to-rust`、`harness-bdd-coding`、`harness-bdd-design` 技能。

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **parity_verifying**(可选,需项目提供 `.opencode/agents/parity-verifier.md`):调用 `parity-verifier` agent(行为等价性检查 + 项目领域不变量)。失败 → Fixing,优先调用 `failure-resolver` agent 诊断。

- **reviewing**(占位符为空则跳过此阶段): 加载 `harness-code-review(mode=full)` 技能。

- evaluating、parity_verifying、reviewing 任一阶段失败 → Fixing(见下)。

## 多 Plan 执行(强制 worktree 隔离)

`plan_list.length > 1` 时,每个 Plan 分配独立 git worktree + sub-agent。

### 流程

#### Phase 1: 并行编码 + 增量检视(每个 worktree 独立)

主 Agent 为每个 Plan 创建 worktree 并并行启动 sub-agent:

```
task(category="deep", load_skills=["c-translate-to-rust", "harness-bdd-coding", "harness-bdd-design"], run_in_background=true,
     workdir="worktrees/{plan_name}",
     prompt="Plan: {plan_path}, module: {module},
             state: .opencode/harness/state/{module}-{plan}-state.json")
```

每个 sub-agent 在自己 worktree 内执行:

```
coding → evaluating → parity_verifying → incremental_reviewing → completing (status=completed)
```

> sub-agent **不执行** full_reviewing(全量审查由主 Agent merge 后统一执行)。

#### Phase 2: 串行 merge

全部 sub-agent 完成后,主 Agent 按 plan_list 顺序逐个 merge:

- 读 worktree 内 plan state → 确认 `status="completed"`
- `git merge feature/{plan}` → 刷新主 state `plan_status[plan]="completed"`
- `git worktree remove worktrees/{plan}`

> merge 是 git 操作,不是 workflow phase。

#### Phase 3: 全局检视(主干)

主 Agent 在主干执行:

```
evaluating → parity_verifying → full_reviewing → completing
```

- evaluating: `harness-code-evaluator` skill(跨 Plan 整体评估)
- parity_verifying: `parity-verifier` agent(可选;跨 Plan 行为等价性扫描,需要项目提供 agent 定义)
- full_reviewing(为空则跳过):

### 详情

- **编码**:加载 `c-translate-to-rust`、`harness-bdd-coding`、`harness-bdd-design` 技能。

- **evaluating**:调用 `harness-code-evaluator` skill(Plan↔代码一致性评估)。失败 → Fixing。

- **incremental_reviewing**(占位符为空则跳过此阶段):加载 `harness-code-review` 技能（mode=incremental）。

- **parity_verifying**(可选,需 `.opencode/agents/parity-verifier.md` 存在):调用 `parity-verifier` agent(行为等价性检查 + 项目领域不变量)。失败 → Fixing。

- **full_reviewing**(占位符为空则跳过此阶段):加载 `harness-code-review(mode=full)` 技能。

- evaluating、parity_verifying、incremental_reviewing、full_reviewing 任一阶段失败 → Fixing(见下)。

> Sub-agent BLOCKED 时主 Agent 通过 `question()` 询问:**跳过该 Plan / 主 Agent 进入修复 / 全部中止**。

## Fixing 阶段

任何 evaluating / parity_verifying / incremental_reviewing / full_reviewing 失败时进入:

### 通用流程(所有触发阶段)

1. **读报告**提取 HIGH / critical / major issue
2. **记录修复尝试**:`attempt_counts[scope_key]` 递增 + `error_signature` + `strategies_tried`
   - `scope_key`:state 的 `plan` 字段非空则为该 plan 路径;否则 `"global"`
3. **加载 fix-self-check skill(若存在)**执行修前检查:因果链诊断 + 爆炸半径论证 + 策略验证。不通过 → 再思考(回退/换策略/拆分),不人工
4. **修复代码**(每次必须换策略;禁相同代码重试)
5. **加载 fix-self-check skill(若存在)**执行修后检查:回归检测 + diff 质量 + 健康趋势。不通过 → 最小粒度回退 + 再思考
6. **重跑失败阶段**,刷新 state: `phase = trigger_stage`, `fixing = null`
7. Max 5 次 → `status = "blocked"` 上报人工

### Parity Verifying 触发时的增强流程(迁移/翻译项目)

当 `trigger_stage = parity_verifying` 且项目中存在 `parity-verifier` agent 与 `failure-resolver` agent 时,在第 2 步与第 3 步之间插入 failure-resolver 诊断:

```
1. 读报告
   ├── 读 evaluator / code-review 报告(如适用)
   └── 读 parity-verifier-review.json(必)
       → 提取 issues 数组(critical + major)
       → 提取 regression_indicators(oscillation 警告)

2. 记录修复尝试(同上)

[NEW] 2.5 调用 failure-resolver agent
     input:
       - parity_issues: issues 数组
       - test_failures: cargo test stderr 摘要
       - prior_attempts: state.attempt_counts[scope_key].strategies_tried
       - spec_dir: spec 目录路径(若存在,含 e2e_spec.md + unit_spec.md + domain_invariants.md)
     output → failure-resolver-result.json 包含:
       - decision: fixed / false_positive / real_gap / inconclusive
       - strategyApplied: 选定的修复策略
       - waiverEvidence: false_positive 时的论证证据

     按 decision 分支处理:
       - fixed          → 继续第 3 步(应用 failure-resolver 选定的策略)
       - false_positive → 记录 waiver 到 state.fixing.waivers[]
                         → 直接重跑 parity_verifying(跳过修复)
                         → attempt_counts 不计入(非真实修复)
       - real_gap       → 标记 needs-review,停止自动修复
                         → `status = "blocked"`,blocked_reason = "real parity gap: ..."
       - inconclusive   → 同上,blocked_reason = "inconclusive: ..."
                         → 升级人工决策

3. 加载 fix-self-check 执行修前检查(同上)

4. 修复代码 — 优先使用 failure-resolver 建议的策略

5. 加载 fix-self-check 执行修后检查(同上)

6. 重跑 parity_verifying 阶段
   → pass:刷新 state,进入下一阶段
   → fail:检查 issues 数是否递减
      ├── 递减 → 继续循环(回到第 1 步)
      ├── 持平或增加 → oscillation 警告
      │   └── 连续 2 次不退化 → `status = "blocked"`,上报人工
      └── 全解决 → pass
```

### 禁止事项(Fixing 阶段补充)

- ❌ 禁止在 failure-resolver 返回 `real_gap` / `inconclusive` 时强行修复 — 这是人工升级信号
- ❌ 禁止把 `false_positive` 用于掩盖真实 parity failure — 必须有充分 waiverEvidence
- ❌ 禁止跳过 `prior_attempts` 检查 — oscillation 检测是 failure-resolver 的核心价值
- ❌ 禁止修改源语言源码 / spec 文件 / golden fixture 来"通过" — 这些都是只读金标准

> fix-self-check 是修复思维方式的门禁,不替代 retry 计数和 state 管理(那些由本 skill 负责)。fix-self-check 只管 agent 怎么想,不管怎么计数。failure-resolver 则专管 parity 类失败的诊断与策略选择。

## State 管理

**所有 state.json 使用统一 schema**,字段按角色(执行 vs 编排)填充 null/非 null。

### 统一 Schema

```json
{
  "module": "{module_name}",
  "plan": "...plan-a.md | null",
  "truth_source_path": ["...plan-a.md"],
  "status": "running | blocked | completed",
  "last_run": "2026-06-26T10:30:00Z",
  "workflow": {
    "phase": "coding | evaluating | parity_verifying | incremental_reviewing | full_reviewing | fixing | completing",
    "current_plan": "...plan-a.md | null",
    "current_task": "... | null",
    "current_skill": "... | null",
    "fixing": null | {
      "trigger_stage": "evaluating | parity_verifying | incremental_reviewing | full_reviewing",
      "reports": [...],
      "parity_issues": ["仅 trigger_stage=parity_verifying 时存在,来自 parity-verifier-review.json 的 issues 摘要"],
      "waivers": ["仅 parity_verifying 触发且 failure-resolver 返回 false_positive 时累积,含 issueId + waiverEvidence"]
    },
    "tasks_completed": ["task_001"],
    "tasks_remaining": ["task_003"],
    "plan_status": null | {
      "...plan-a.md": "pending | running | completed | blocked"
    },
    "attempt_counts": {
      "...plan-a.md | global": {"count": 0, "error_signature": null, "strategies_tried": []}
    }
  }
}
```

### 三种 state 文件的角色

| state 文件 | 路径 | `plan` 字段 | `plan_status` 字段 | 写入者 |
|-----------|------|-------------|-------------------|--------|
| 单 Plan 主 state | `.opencode/harness/state/{module}-workflow-state.json` | 唯一 plan 路径 | null | 主 Agent |
| 多 Plan 主 state | 同上 | null | 含所有 plan 的 map | 主 Agent |
| 多 Plan 执行 state | `worktrees/{plan}/.opencode/harness/state/{module}-{plan}-state.json` | 当前 plan 路径 | null | sub-agent |

### 字段语义

| 字段 | 取值 | 说明 |
|------|------|------|
| `plan` | 路径/null | 当前 state 归属的 Plan;编排 state 为 null |
| `plan_status` | null/map | 仅多 Plan 主 state 非空,记录每个 plan 的编排进度 |
| `current_plan` | 路径/null | 正在执行的 Plan;编排 state / Global 阶段为 null |
| `current_task` | task_id/null | 编码中的 task;非编码阶段为 null |
| `fixing.trigger_stage` | evaluating/parity_verifying/incremental_reviewing/full_reviewing | 触发 fixing 的阶段;parity_verifying 仅当项目提供 parity-verifier agent 时出现 |
| `fixing.reports` | 路径列表 | 失败报告路径(evaluator/code-review/parity-verifier) |
| `fixing.parity_issues` | issues 摘要数组 | 仅 trigger_stage=parity_verifying;来自 parity-verifier-review.json 的 critical/major issues |
| `fixing.waivers` | waiver 数组 | 仅 parity_verifying 触发 + failure-resolver false_positive 决策;累计不可重放的"误报豁免" |
| `attempt_counts[key]` | plan 路径/"global" | 对应 scope 的修复次数记录;false_positive 不计入 |

## Phase 状态机与 State 刷新时机

**每个 phase 的进入/退出都必须刷新 state.json。下表是唯一的刷新规则来源,其他段落不再重复。**

| Phase | 触发 | 退出 | 进入时刷新 | 退出时刷新 |
|-------|------|------|-----------|-----------|
| `coding` | plan 启动 / 上一 task 完成 | 全部完成 → evaluating | phase="coding" | 每 task: tasks_completed += / tasks_remaining -= / current_task = next |
| `evaluating` | coding 完成 或 merge 完成(多 plan) | 通过 → parity_verifying(若项目提供 parity-verifier agent)或 incremental_reviewing;失败 → fixing | phase="evaluating", current_skill="evaluator" | 通过: current_skill=null; 失败: phase="fixing" + fixing={trigger_stage, reports} + attempt_counts++ |
| `parity_verifying` | evaluating 通过(仅当项目提供 `.opencode/agents/parity-verifier.md` 时存在) | 通过 → reviewing(单 plan)或 incremental_reviewing(worktree)或 full_reviewing(多 plan 主干);失败 → fixing | phase="parity_verifying", current_skill="parity-verifier" | 通过: current_skill=null; 失败: phase="fixing" + fixing={trigger_stage:"parity_verifying", reports, parity_issues} + attempt_counts++ |
| `incremental_reviewing` | evaluating/parity_verifying 通过 | 通过 → full_reviewing(单 plan) 或 completing(worktree);失败 → fixing | phase="incremental_reviewing" | 通过: current_skill=null; 失败: phase="fixing" + fixing={...} + attempt_counts++ |
| `full_reviewing` | incremental_reviewing 通过(单 plan) 或 merge 完成(多 plan 主干) | 通过 → completing;失败 → fixing | phase="full_reviewing" | 通过: current_skill=null; 失败: phase="fixing" + fixing={...} + attempt_counts++ |
| `fixing` | 任一 fail | 修完 → 返回 trigger_stage;超限/real_gap/inconclusive → blocked | phase="fixing", fixing={...} | 修完: phase=trigger_stage, fixing=null; 超限/real_gap: status="blocked", blocked_reason={...} |
| `completing` | full_reviewing 通过 或 incremental_reviewing 通过(worktree 无 full) | status="completed" | phase="completing", current_plan=null, current_task=null, current_skill=null | status="completed" |

### 多 Plan 编排阶段的额外注意事项

| 时机 | 写入字段 |
|------|---------|
| 创建 worktree + 启动 sub-agent | `plan_status[plan]="running"` |
| Sub-agent 完成(读其 plan state 后) | `plan_status[plan]="completed"` 或 `"blocked"` |
| 全部 plan_status="completed" 后 | 主 Agent 串行 git merge,逐个将 `plan_status[plan]="completed"`(merge 成功后保持);全部完成后进入 evaluating |

> **核心约束**:AI 不调 Write 工具,state.json 就不会更新。续传完全依赖 state 文件,上表每个时机都必须刷新。

### 状态恢复

1. **Read state.json**:不存在 → 全新启动,按 `plan_list.length` 决定角色(单 Plan:执行;多 Plan:编排)
2. **识别角色**:`plan_status` 是否为 null → 执行状态(false) / 编排状态(true)
3. **status 分支**:`blocked` → `question()` 上报;`completed` → 直接结束响应(不 question)
4. **按 phase 恢复**:
   - 执行状态:`phase + current_plan + current_task` → 续传该 task 或进入相应 phase
   - 编排状态:`plan_status` 中 status="running" 的 plan → 重启对应 sub-agent;全部 completed → 进入 evaluating

## 禁止事项

1. ❌ 修改 Plan / 测试文件来"通过"(含删除失败测试)
2. ❌ 不读错误输出就重试,或用相同代码重试
3. ❌ Task 级执行 evaluating/incremental_reviewing/full_reviewing/fixing(只在 phase 层级)
4. ❌ AI 自评替代门禁(必须执行命令或委托 sub-agent)
5. ❌ 漏刷新 state.json(上表每个时机必须全部执行)
6. ❌ 多 Plan 主 Agent 直接编码(必须 sub-agent + worktree)
7. ❌ 跳过 worktree(每个 Plan 必须独立 worktree,禁共享主干)
8. ❌ 并发 merge 到主干(必须按 plan_list 顺序串行)
9. ❌ Sub-agent 写主 state.json(只写自己 worktree 内的 plan state)
10. ❌ Merge 前未确认 plan state.json 的 status="completed"
11. ❌ 修改 `truth_source_path` 中的 Plan 路径数组
12. ❌ 除 `status=blocked` 终态外,任何 phase 中间禁止调用 `question()` 向用户提问
13. ❌ 禁止在 phase 切换时停下"确认" — state machine 定义了自动流转条件,满足即流转
14. ❌ 禁止在 build/test 失败后 `question()` — 必须自动进入 Fixing 阶段
15. ❌ 禁止在 evaluating/incremental_reviewing/full_reviewing 失败后 `question()` — 必须自动进入 Fixing 阶段
16. ❌ 禁止在 Fixing 修复后 `question()` — 必须自动返回 trigger_stage 重跑
17. ❌ 禁止自行判定"需要澄清需求"而停下 — Plan 是唯一 truth source,按 Plan 执行
18. ❌ 禁止在未刷新 state.json 为 `status=completed` 前声称"已完成"
19. ❌ 禁止在 coding 阶段 task 之间 `question()` — 按 `tasks_remaining` 自动推进
20. ❌ 多 Plan 场景:sub-agent 完成后主 Agent 禁止 `question()` — 必须自动进入 merge 流程
