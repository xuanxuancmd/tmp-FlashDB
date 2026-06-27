---
name: harness-translate-code-review
description: 翻译项目的检视编排技能。并行触发通用检视Agent和翻译忠实度检视Agent，合并结果，CatA/B分类处理翻译差异，循环修复直到通过。
---

# 翻译项目检视编排 Skill

## 适用范围（硬约束）

**仅限其他语言到 Rust 的翻译项目。** 本 Skill 和 `translate-code-review-agent` 的所有比对逻辑、CatA/CatB 分类规则、Rust 特有检查项均基于"目标语言为 Rust"的前提设计。如目标语言非 Rust，本 Skill 不适用，需另行设计。

## 职责

指导主Agent编排两个检视SubAgent的并行执行与结果消费：

- **code-review-agent**：通用检视（构建/测试/lint/AI检查/占位符检测）
- **translate-code-review-agent**：翻译忠实度检视（1:1结构对等/语义忠实度/图谱对等）

两个Agent无数据依赖，可并行触发。主Agent读取两者的summary.json，任一pass=false即进入修复循环（最多3次）。翻译差异问题需按CatA/B分类处理：CatA需用户审批，CatB直接修复。

## Evidence消费原则

### 原则1：双summary.json是决策入口

两个Agent各产出一个summary文件，必须全部读取：

- 通用检视：`<module>-review-summary.json`（来自code-review-agent）
- 翻译检视：`<module>-translate-review-summary.json`（来自translate-code-review-agent）

判定规则：
- **两个summary均pass=true** → 任务完成 ✅
- **任一pass=false** → 进入修复循环，从各自 `blocking_issues[]` 提取阻断性问题

### 原则2：按需读取evidence文件

summary.json中每个检查项含evidence_file路径。当检查项pass=false时，Read对应evidence文件定位具体问题：

| 检查项失败 | 来源Agent | 读取文件 | 处理方式 |
|-----------|----------|---------|---------|
| parity_check / graph_parity_check | translate | parity.json / graph-parity.json | CatA/B分类（见"Parity差异分类"） |
| fidelity_check | translate | translate-fidelity.json | CatA/B分类（见"Parity差异分类"） |
| placeholder_detection | code-review | placeholder.json | 直接修复（CatB） |
| build_check | code-review | build-result.txt | 定位编译错误，修复 |
| test_check | code-review | test-result.txt | 定位失败用例，修复 |
| AI检查（零容忍） | code-review | ai-check-test.json / ai-diff.json | 直接修复 |

### 原则3：report.md作为补充阅读

两份报告提供详细分析（人阅读视角）：

- `<module>-review-report.md` — 通用检视详情（构建/测试/lint分析、BUG追踪、修复建议）
- `<module>-translate-review-report.md` — 翻译忠实度比对详情（结构差异、语义偏差、图谱对比）

当evidence文件信息不足以定位问题时，Read对应report.md补充上下文。

## 检视触发

两个Agent无数据依赖，可并行触发：

```
# 通用检视（code-review-agent）
task(
  subagent_type="code-review-agent",
  description="通用检视",
  prompt="检视路径: {review_path}"
)

# 翻译忠实度检视（translate-code-review-agent）
task(
  subagent_type="translate-code-review-agent",
  description="翻译检视",
  prompt="模块: {module}
          检视路径: {review_path}
          源码根目录: {source_root}
          graphify可用: {graphify_available}"
)
```

**参数说明**：

| 参数 | code-review-agent | translate-code-review-agent | 说明 |
|------|:-:|:-:|------|
| `module` | ✗ | ✓ | 被检视的模块名称 |
| `review_path` | ✓ | ✓ | 被检视的代码路径 |
| `source_root` | ✗ | ✓ | Kafka Java源码根目录（用于1:1比对） |
| `graphify_available` | ✗ | ✓ | 图谱工具是否可用（true/false） |

两个Agent完成后各自返回报告路径，主Agent用Read工具读取。

## 循环修复流程

1. **并行启动**两个检视Agent → 等待两者均完成
2. Read两个summary.json（原则1）→ 判断是否都pass
3. **Both pass=true** → 任务完成，交付用户
4. **Any pass=false** → 按需Read evidence文件（原则2）→ 定位问题并修复：
   - 来自code-review-agent的问题 → 按通用修复流程处理
   - 来自translate-agent的parity/fidelity问题 → 按CatA/B分类处理（见下方）
   - 修复后若只需重跑某个Agent（如仅translate有问题），可单独重跑该Agent
5. 修复后重新检视（retry += 1）
6. 达到3次仍未通过 → 向用户报告失败，列出所有未解决问题（区分HIGH/LOW、CatA/CatB），请求介入

## Parity差异分类（CatA/B）

对translate-agent报告的parity/fidelity issue，必须逐条分类：

**Category A — 语言/框架差异型**（无法1:1翻译，需用户审批后在规格清单中标注ignore）:
- 异常→enum、反射/泛型/ClassLoader无对应
- 三方API签名差异（如Java SDK方法在Rust生态无对等签名）
- 接口默认方法（Java interface default → Rust trait无法直接映射）
- 序列化→serde签名不可1:1（如Java Serializable vs serde trait bound）

**Category B — 可修复型**（直接参考Kafka源码1:1修复代码）:
- struct/trait缺失、函数签名错误
- 逻辑遗漏（条件分支/边界处理未翻译）
- Java内部类 → Rust应平铺为独立文件但未做

**修复策略（优先级×分类交叉矩阵）**：

| | Category A | Category B |
|---|-----------|-----------|
| **HIGH** | 用question()向用户提ignore申请；同意→标注规格清单(ignore+ignore_reason)；不同意→降级为Category B修复 | 立即修复代码 |
| **LOW** | 暂不处理 | 暂不处理 |

**CatA审批流程**：
1. 向用户说明差异原因、影响范围、建议的ignore理由
2. 用户同意 → 在规格清单中标注 `ignore: true` + `ignore_reason: "..."`
3. 用户不同意 → 降级为Category B，尝试用Rust特性实现1:1等价

## 检视流程图

```
┌───────────────────────────────────────────────────────────────┐
│   翻译检视-修复循环 (max_retries=3)                             │
├───────────────────────────────────────────────────────────────┤
│                                                               │
│  current_retry = 0                                            │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              并行启动两个Agent                             │ │
│  │  ┌─────────────────┐  ┌───────────────────┐            │ │
│  │  │ code-review      │  │ translate-code-   │            │ │
│  │  │ -agent           │  │ review-agent      │            │ │
│  │  │ (build/test/lint)│  │ (parity/fidelity) │            │ │
│  │  └────────┬─────────┘  └────────┬──────────┘            │ │
│  │           └──────┬──────────────┘                        │ │
│  │                  ↓                                       │ │
│  │  Read 两个 summary.json → 都pass?                        │ │
│  │      ├─ YES → 任务完成 ✅                                 │ │
│  │      └─ NO  → evidence读取 → CatA/B分类                  │ │
│  │              ↓                                            │ │
│  │  CatB: HIGH → 立即修复                                   │ │
│  │  CatA: HIGH → question()审批 → ignore / 降级CatB         │ │
│  │              ↓                                            │ │
│  │  修复完成 → current_retry += 1 → 返回循环开始             │ │
│  │                                                           │ │
│  └─────────────────────────────────────────────────────────┘ │
│                                                               │
│  max_retries达标 → 报告失败，请求用户介入                       │
│  列出: HIGH阻断 + LOW非阻断 + CatA未审批 + CatB未修复          │
│                                                               │
└───────────────────────────────────────────────────────────────┘

交叉决策矩阵:
  HIGH×CatB → 立即修复     | HIGH×CatA → 立即提ignore申请
  LOW×CatB  → 延后修复     | LOW×CatA  → 延后提ignore申请

🔒 用户审批门: Category A的ignore标注必须经过用户同意
```

## 禁止事项

1. ❌ 直接执行校验脚本（应由SubAgent执行）
2. ❌ 忽略HIGH blocking_issues继续其他工作
3. ❌ 超3次循环后继续自动重试
4. ❌ 未通过检视时跳过修复直接交付
5. ❌ 未经用户同意修改规格清单标注ignore
6. ❌ 将Category B当作Category A逃避修复
7. ❌ 将Category A直接修复而不申请ignore
8. ❌ 只跑一个Agent而跳过另一个

## 强制事项

1. ✅ 两个Agent必须都执行（不可跳过任一个）
2. ✅ 区分HIGH/LOW优先级，HIGH阻断时skip其他subAgent
3. ✅ 遵循3次循环上限
4. ✅ parity issue必须分类CatA/B，CatA需question()审批
5. ✅ 标注ignore时必须填写ignore_reason
6. ✅ 用户不同意CatA → 降级为CatB尝试1:1修复
7. ✅ 两个summary.json都pass=true才算整体通过

## SubAgent详细定义

两个SubAgent的执行流程、权限、报告格式：

- 通用检视SubAgent → `.opencode/agents/code-review-agent.md`
- 翻译检视SubAgent → `.opencode/agents/translate-code-review-agent.md`
