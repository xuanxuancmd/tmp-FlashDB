# Harness 工程目录

本目录存放项目 harness 工程的**运行工具与运行结果**，由编码闭环 workflow 及 AI 自动读写。

## 目录说明

| 子目录 | 用途 | 写入方 |
|--------|------|--------|
| `state/` | workflow 状态 JSON（`fdb-workflow-state.json`） | workflow skill 写，`state-guard.py` hook 校验 |
| `logs/` | 运行日志（`fdb-run-log.md`） | hook 脚本自动追加 |
| `scripts/` | 运行工具（`state-guard.py`、自定义 linter 等跨平台脚本） | meta skill 写入 |
| `tmp/` | 非 Plan 上下文持久化（`truth_source` 纯文本需求、GitHub Issue 抓取结果等） | workflow 在启动时写入，编码期间读取 |
| `manifests/` | 黄金清单（`*.golden.yaml`，翻译类项目） | `build_golden_manifest.py` 生成 |
| `features/` | BDD Gherkin 场景（`*.feature`） | `harness-bdd-design` skill 生成 |
| `evidence/` | 校验输出（`*.json` / `*.md`） | evaluator / reviewer agent |
| `specs/` | 逆向验收规格（`*-spec.md`） | `reverse-engineering-test-spec` skill 生成 |
| `ignores/` | 忽略规则（`*-ignores.yaml`） | `harness-code-review` skill 追加 |

## 约束

- **禁止手动编辑** `state/*.json`（由 hook 校验 schema）
- **禁止手动编辑** `logs/*.md`（仅 hook 追加）
- **禁止手动编辑** `manifests/*.golden.yaml`（仅工具生成；追加 `ignore` 需用户审批）
- **禁止手动编辑** `features/*.feature`（编码阶段只读）
- `scripts/` 内脚本跨平台优先（Python > shell），不依赖特定工具（如 jq、bash）
