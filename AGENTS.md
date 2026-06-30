# 项目介绍

当前项目完成开元FlashDB从 C 到 Rust 的 1:1 翻译，确保规格不遗漏，同时遵从rust语言特点。

FlashDB 源码参考：`C:\wanglong\temp\FlashDB`。

## Harness 工程目录

项目使用 `.opencode/harness/` 目录管理编码闭环工程相关的运行工具与产物：

| 子目录 | 说明 |
|--------|------|
| `state/` | **核心** workflow 持久化状态（断点续传） |
| `scripts/` | 校验脚本（`state-guard.py`、`loop-orchestrator.ts` 等，跨平台） |
| `logs/` | **核心** 运行日志（由 hook 脚本自动追加） |
| `evidence/` | 校验输出产物（`*.json`） |
| `features/` | BDD Gherkin 场景（`*.feature`） — `harness-bdd-design` skill 生成 |

完整说明见 `.opencode/harness/README.md`。

