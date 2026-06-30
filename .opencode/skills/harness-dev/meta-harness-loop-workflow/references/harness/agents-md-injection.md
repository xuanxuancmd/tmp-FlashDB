# AGENTS.md 注入段落模板

> Step 3.3 用此模板在 `AGENTS.md` 末尾追加 harness 目录说明段落
> **跨平台**：本段内容平台无关，OpenCode / Claude Code 通用
> 变量: `{harness_dir}` = 项目 harness 目录相对路径（默认 `.opencode/harness`）

---

## Harness 工程目录

项目使用 `{harness_dir}/` 目录管理编码闭环工程相关的运行工具与产物：

| 子目录 | 说明 |
|--------|------|
| `features/` | BDD Gherkin 场景（`*.feature`） | `harness-bdd-design` skill 生成 |
| `e2e/` | e2e的测试用例设计） | `generate-e2e-test-guide` skill 追加 |
| `state/` | **核心** workflow 持久化状态（断点续传） |
| `scripts/` | 校验脚本（`state-guard.py` 等，跨平台） |
| `logs/`  | **核心** 运行日志（由 hook 脚本自动追加） |

完整说明见 `{harness_dir}/README.md`。

---

## 注入规则

1. 扫描 `AGENTS.md` 是否已包含关键词 `## Harness` 或 `.opencode/harness`
2. **已存在** → **覆盖**对应段落（保持与模板一致，不重复注入）
3. **不存在** → 在文件末尾追加上方段落
4. **若 AGENTS.md 只读或用户拒绝** → 在输出中提示用户手动添加
