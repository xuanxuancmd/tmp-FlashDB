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
 *    - 日志写入文件（副作用，不回传 Agent）
 *    - 仅对主 state（{module}-workflow-state.json）输出 [TODO]（脚本内部 isMainState 判断）
 *      AI 据此调 TodoWrite 刷新；per-plan state 写入只记日志，不触发 todo 刷新
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
        // 日志写入文件（副作用），仅主 state 输出 [TODO] 到 stdout（脚本内部 isMainState 判断）
        // per-plan state 写入只记日志，stdout 为空，不触发主 Agent todo 刷新
        const postWriteScript = path.join(scriptDir, "workflow-todo-write.js")

        const postProc = await $`node ${postWriteScript} ${filePath}`.nothrow()
        const postOutput = postProc.stdout.toString().trim()
        const postStderr = postProc.stderr.toString().trim()

        if (postOutput) {
          // stdout 含 [LOG] 和 [TODO] 行，直接注入提示
          output.output += `\n\n📋 ${postOutput}`
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
