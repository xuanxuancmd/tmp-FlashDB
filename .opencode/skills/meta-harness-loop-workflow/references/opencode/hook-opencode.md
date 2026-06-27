# OpenCode 治理 Hook 模板（loop-governance.ts）

> 生成项目级唯一的流程治理 hook，监听所有 workflow 的 state 文件写入事件。
> **不再为每个 workflow 生成独立 hook** — 一个 `loop-governance.ts` 治理所有 `*-workflow-state.json`。
> 调用跨平台 Python 版 `state-guard.py`（替代旧的 shell 版），不依赖 jq/bash。

---

```typescript
import { Plugin } from "@opencode-ai/plugin"
import { $ } from "bun"
import path from "path"
import fs from "fs/promises"

/**
 * Loop Governance Plugin — 迁移工作流流程治理
 *
 * 职责（AI 无法绕过）：
 * 1. state.json 写入后自动运行 state-guard.py（schema 校验 + 重试上限）
 *    - exit 0  → ✅ 通过
 *    - exit 1  → 🛑 阻断（schema 损坏 / 旧 schema 重试超限）
 *    - exit 2  → 🛑 阻断（新 schema 任务尝试上限）
 * 2. state.json 写入后自动追加结构化运行日志到 {project_dir}/.opencode/harness/logs/{module}-run-log.md
 *
 * 与 harness-validator.ts 的关系（如存在）：
 * - harness-validator: 代码质量守卫（placeholder 检测、manifest parity、cargo check）
 * - 本 hook:          流程治理守卫（状态完整性、重试上限、运行日志）
 *
 * 与 workflow skill 的关系：
 * - Skill（知识层）: 告诉 AI "怎么做"
 * - Hook（控制层）: 强制执行 — 即使 AI 绕过 Skill，hook 仍会校验
 */
export const LoopGovernancePlugin: Plugin = async (ctx) => {
  const scriptDir = path.join(ctx.directory, ".opencode/harness/scripts")
  const logDir = path.join(ctx.directory, ".opencode/harness/logs")

  return {
    tool: {},

    "tool.execute.after": async (input, output) => {
      if (input.tool !== "edit" && input.tool !== "write") {
        return
      }

      const filePath = input.args?.filePath || ""
      const normalizedFilePath = filePath.split(path.sep).join("/")

      // ========== Check 1: state.json 写入 → 运行 guard + 追加日志 ==========
      // 监听所有 .opencode/harness/state/*-workflow-state.json
      if (
        normalizedFilePath.includes(".opencode/harness/state/") &&
        normalizedFilePath.endsWith("-workflow-state.json")
      ) {
        // ---- 1a. 运行 state-guard.py ----
        const guardScript = path.join(scriptDir, "state-guard.py")

        const guardProc = await $`python ${guardScript} ${filePath}`.nothrow()
        const guardOutput = guardProc.stdout.toString().trim()
        const guardStderr = guardProc.stderr.toString().trim()

        if (guardOutput) {
          output.output += `\n\n🔄 LOOP GUARD: ${guardOutput}`
        }

        if (guardProc.exitCode === 1 || guardProc.exitCode === 2) {
          // 故意阻断：schema 损坏 / 重试超限 / 任务尝试上限
          output.output += `\n\n🛑 LOOP GUARD BLOCKED: ${guardStderr || guardOutput}`
          output.output += `\n💡 必须暂停循环并 question() 上报人工`
        } else if (guardProc.exitCode !== 0) {
          // 脚本自身异常（Python 缺失 / 权限问题）— 警告但不阻断
          output.output += `\n\n⚠️ LOOP GUARD WARNING: state-guard.py 异常退出 (code ${guardProc.exitCode})`
          if (guardStderr) {
            output.output += `\n   stderr: ${guardStderr.slice(0, 300)}`
          }
          output.output += `\n💡 请检查 Python 环境，循环继续但失去 state 校验`
        }

        // ---- 1b. 追加结构化运行日志 ----
        try {
          const stateContent = await fs.readFile(filePath, "utf-8")
          const state = JSON.parse(stateContent)
          const moduleName = state.module || "unknown"
          const logFile = path.join(logDir, `${moduleName}-run-log.md`)

          await fs.mkdir(logDir, { recursive: true })

          const now = new Date()
          const pad = (n: number, w = 2) => String(n).padStart(w, "0")
          const timestamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`
          const runId = state.last_run || "unknown"

          // 兼容 NEW schema（workflow 对象）和 OLD schema（phase / retry_count）
          let summary: string
          if (state.workflow) {
            const wf = state.workflow
            const currentPlan = wf.current_plan || "none"
            const currentTask = wf.current_task || "none"
            const completed = wf.tasks_completed?.length || 0
            const remaining = wf.tasks_remaining?.length || 0
            // 统计所有 attempt_counts 中的最大 count
            const attempts = wf.attempt_counts || {}
            const maxAttempt = Object.values(attempts).reduce(
              (max: number, e: any) => Math.max(max, e?.count || 0), 0
            )
            summary = `
## Run ${runId} (${timestamp})
- Schema: workflow-based
- Status: ${state.status || "unknown"}
- Phase: ${wf.phase || "unknown"}
- Current plan: ${currentPlan}
- Current task: ${currentTask} (completed: ${completed}, remaining: ${remaining})
- Max attempts: ${maxAttempt}/3
- Fixing: ${wf.fixing ? wf.fixing.trigger_stage : "none"}
`
          } else {
            summary = `
## Run ${runId} (${timestamp})
- Schema: phase-based
- Phase: ${state.next_phase || "unknown"}
- Status: ${state.status || "unknown"}
- Retry: evaluator=${state.retry_count?.evaluator || 0}/3, reviewer=${state.retry_count?.reviewer || 0}/3
- Last completed: ${state.last_completed_phase || "none"}
`
          }

          await fs.appendFile(logFile, summary, "utf-8")
        } catch (e) {
          output.output += `\n⚠️ LOOP GUARD: 运行日志追加失败: ${e}`
        }

        return
      }
    },
  }
}
```

---

---

## 生成规则

1. **文件名固定**：`loop-governance.ts`
2. **项目级唯一**：无论生成多少个 workflow，整个项目只保留一个 `loop-governance.ts`
3. **已有则检查 schema 兼容性**：已存在的 `loop-governance.ts` 若未支持 task-based schema → 更新；否则跳过
4. **配套 state-guard.py**：同时复制到 `.opencode/harness/scripts/state-guard.py`（从 `references/scripts/state-guard-template.py`）
5. **自动注册**：生成后**自动将** `.opencode/plugins/loop-governance.ts` 追加到 `.opencode/opencode.json` 的 `plugin` 数组（已存在则跳过）
6. **重启提示**：生成后提示用户重启 OpenCode 以加载/更新 plugin
