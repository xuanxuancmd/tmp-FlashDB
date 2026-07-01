# OpenCode 治理 Hook 模板（loop-governance.ts）

> 生成项目级唯一的流程治理 hook，监听所有 workflow 的 state 文件写入事件。
> **不再为每个 workflow 生成独立 hook** — 一个 `loop-governance.ts` 治理所有 `*-workflow-state.json`。
> 调用跨平台 Python 版 `state-guard.py`（schema 校验）+ Node.js 版 `workflow-todo-write.js`（todo 计算 + 日志），不依赖 jq/bash。

---

```typescript
import { Plugin } from "@opencode-ai/plugin"
import { $ } from "bun"
import path from "path"

/**
 * Loop Governance Plugin — 迁移工作流流程治理
 *
 * 职责（AI 无法绕过）：
 * 1. state.json 写入后自动运行 state-guard.py（schema 校验 + 重试上限）
 *    - exit 0  → ✅ 通过
 *    - exit 1  → 🛑 阻断（schema 损坏 / 旧 schema 重试超限）
 *    - exit 2  → 🛑 阻断（新 schema 任务尝试上限）
 * 2. state.json 写入后自动运行 workflow-todo-write.js（todo 计算 + 日志追加）
 *    - 日志写入文件（副作用，不回传 Agent）：运行日志 + todo 调用记录
 *    - 仅对主 state（{module}-workflow-state.json）输出纯 todos[] JSON 到 stdout，
 *      hook 直接注入 tool output，AI 据此调 TodoWrite 刷新；
 *      per-plan state 写入只记日志，stdout 为空，不触发 todo 刷新
 *    - 依赖 .opencode/harness/workflow.yaml：脚本从中读取 stage 顺序、on_failure 跳转、
 *      skill 映射，动态推导 todo 项（每 plan 1 项，不做全景投影）
 *
 * 降级机制：
 * - 本 hook 是"自动触发"层，依赖运行时支持 hooks
 * - 不支持 hooks 时，由 workflow skill 指示 AI 在写 state.json 后手动调用
 *   `node workflow-todo-write.js {path}`（meta skill 生成 workflow 时判断 hooks 可用性，
 *   不可用则在 workflow 模板插入脚本调用指令）
 * - state-guard.py（python）和 workflow-todo-write.js（node）是同一套脚本，hook 和 AI 手动调用逻辑一致
 *
 * 与 harness-validator.ts 的关系（如存在）：
 * - harness-validator: 代码质量守卫（placeholder 检测、manifest parity、cargo check）
 * - 本 hook:          流程治理守卫（状态完整性、重试上限、运行日志、todo 投影）
 *
 * 与 workflow skill 的关系：
 * - Skill（知识层）: 告诉 AI "怎么做"
 * - Hook（控制层）: 强制执行 — 即使 AI 绕过 Skill，hook 仍会校验
 */
export const LoopGovernancePlugin: Plugin = async (ctx) => {
  const scriptDir = path.join(ctx.directory, ".opencode/harness/scripts")

  return {
    tool: {},

    "tool.execute.after": async (input, output) => {
      if (input.tool !== "edit" && input.tool !== "write") {
        return
      }

      const filePath = input.args?.filePath || ""
      const normalizedFilePath = filePath.split(path.sep).join("/")

      // ========== state.json 写入 → 运行 guard + post-write ==========
      if (
        normalizedFilePath.includes(".opencode/harness/state/") &&
        normalizedFilePath.endsWith("-state.json")
      ) {
        // ---- 1a. 运行 state-guard.py（schema 校验 + 重试上限）----
        const guardScript = path.join(scriptDir, "state-guard.py")

        const guardProc = await $`python ${guardScript} ${filePath}`.nothrow()
        const guardOutput = guardProc.stdout.toString().trim()
        const guardStderr = guardProc.stderr.toString().trim()

        if (guardOutput) {
          output.output += `\n\n🔄 LOOP GUARD: ${guardOutput}`
        }

        if (guardProc.exitCode === 1 || guardProc.exitCode === 2) {
          output.output += `\n\n🛑 LOOP GUARD BLOCKED: ${guardStderr || guardOutput}`
          output.output += `\n💡 必须暂停循环并 question() 上报人工`
        } else if (guardProc.exitCode !== 0) {
          output.output += `\n\n⚠️ LOOP GUARD WARNING: state-guard.py 异常退出 (code ${guardProc.exitCode})`
          if (guardStderr) {
            output.output += `\n   stderr: ${guardStderr.slice(0, 300)}`
          }
          output.output += `\n💡 请检查 Python 环境，循环继续但失去 state 校验`
        }

        // ---- 1b. 运行 workflow-todo-write.js（todo 计算 + 日志追加）----
        // 日志写入文件（副作用），仅主 state 输出纯 todos[] JSON 到 stdout（脚本内部 isMainState 判断）
        // per-plan state 写入只记日志，stdout 为空，不触发主 Agent todo 刷新
        // stdout 是纯 JSON 数组，直接注入 tool output，AI 据此调 TodoWrite
        const postWriteScript = path.join(scriptDir, "workflow-todo-write.js")

        const postProc = await $`node ${postWriteScript} ${filePath}`.nothrow()
        const postOutput = postProc.stdout.toString().trim()
        const postStderr = postProc.stderr.toString().trim()

        if (postOutput) {
          // stdout 是纯 todos[] JSON 数组，直接注入（不加前缀/提示语，避免噪声）
          output.output += `\n${postOutput}`
        }

        if (postProc.exitCode !== 0) {
          // post-write 失败不阻断主流程（日志和 todo 是辅助功能），仅告警
          output.output += `\n⚠️ LOOP GUARD: workflow-todo-write.js 异常退出 (code ${postProc.exitCode})`
          if (postStderr) {
            output.output += `\n   stderr: ${postStderr.slice(0, 300)}`
          }
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
3. **覆盖策略**：已存在的 `loop-governance.ts` 直接覆盖（保持与模板一致）
4. **配套脚本**：同时复制两个脚本到 `.opencode/harness/scripts/`：
   - `state-guard.py`（从 `references/scripts/state-guard-template.py`）— 依赖 `.opencode/harness/workflow.yaml` 动态推导 VALID_STAGES
   - `workflow-todo-write.js`（从 `references/scripts/workflow-todo-write.js`）— 依赖 `.opencode/harness/workflow.yaml` 读取 stage 顺序和 skill 映射
   - **前置条件**：`.opencode/harness/workflow.yaml` 必须已生成（由 meta skill Step 6.0 实例化），否则两个脚本均无法工作
5. **自动注册**：生成后**自动将** `.opencode/plugins/loop-governance.ts` 追加到 `.opencode/opencode.json` 的 `plugin` 数组（已存在则跳过，避免重复条目）
6. **重启提示**：生成后提示用户重启 OpenCode 以加载/更新 plugin
