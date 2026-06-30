# Claude Code Stop Hook Settings 配置

> Step 10.4 用此模板配置 `.claude/settings.json` 的 Stop 事件 hook
> 仅在 `detected_platforms` 包含 `"claudecode"` 时生成

---

## settings.json 配置内容

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "python .claude/hooks/stop-guard.py",
            "timeout": 15,
            "statusMessage": "Checking completion..."
          }
        ]
      }
    ]
  }
}
```

## 合并规则

1. `.claude/settings.json` 不存在 → 创建并写入（含 `$schema`）
2. 已存在且无 `hooks` 键 → 添加 `hooks` 键
3. 已存在且有 `hooks` 键：
   - 同 event（如已有 `Stop`）→ 在数组末尾追加
   - 不同 event → 新增键
4. 已存在相同 `command` 的 hook → 覆盖（保持与模板一致）

## 独有价值

当 AI 声称完成但 state 显示未完成时，**强制阻止 Claude 结束响应**，迫使它继续工作。

OpenCode 无法原生实现此能力（`session.idle` 无法拦截），只能在 workflow skill 中用 Kill Switch 文字约束。

## 模板变量

- `{state_file_path}` → `.opencode/harness/state/{module}-workflow-state.json`（用于 `stop-guard.py` 读取）
