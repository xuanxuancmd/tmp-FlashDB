#!/usr/bin/env python3
"""
stop-guard-claudecode.py — 编码闭环完成阻断守卫 Hook (Claude Code 专用)

触发：Claude Code Stop event（Claude 完成响应时）
输入：JSON on stdin（Claude Code hook input format）
输出：stdout JSON（decision + reason）
退出码：0 = 正常（JSON 控制放行/阻断）/ 2 = 阻断（stderr 作为 reason 回传）

这是 Claude Code 独有的控制能力：在 AI 声称完成时强制让它继续工作。
OpenCode 无法原生实现此能力，只能在 Skill 中用文字约束 + BLOCKED 状态提示。
"""

import json
import os
import sys


def write_output(msg: str) -> None:
    """统一输出，刷新缓冲区确保 Claude Code hook 及时读取"""
    print(msg, flush=True)


def block(reason: str) -> None:
    """输出 block decision JSON"""
    write_output(json.dumps({"decision": "block", "reason": reason}))


def main() -> int:
    # ========== 读取 Claude Code Stop hook input ==========
    try:
        raw_input = sys.stdin.read()
        hook_input = json.loads(raw_input) if raw_input.strip() else {}
    except (json.JSONDecodeError, UnicodeDecodeError):
        # 无法解析 input — 放行（不要因 hook 错误阻断 Claude）
        return 0

    # ========== Critical: break infinite loop ==========
    # 如果已经在 forced continuation 中（stop_hook_active=true），放行以阻断死循环
    if hook_input.get("stop_hook_active") is True:
        return 0

    cwd = hook_input.get("cwd", "")
    state_rel_path = "{state_file_path}"  # 模板变量：如 .opencode/harness/state/{module}-workflow-state.json
    state_path = os.path.join(cwd, state_rel_path) if cwd else state_rel_path

    # ========== 1. state.json 是否存在 ==========
    if not os.path.isfile(state_path):
        # 没有 state 文件 = 全新启动或首次运行，放行
        return 0

    # ========== 读取 state.json ==========
    try:
        with open(state_path, "r", encoding="utf-8") as f:
            state = json.load(f)
    except (OSError, json.JSONDecodeError) as e:
        # state 损坏或不可读 — 放行并打印警告（让 loop-governance 或 state-guard.py 去阻断）
        sys.stderr.write(f"stop-guard.py: 无法读取 state: {e}\n")
        return 0

    if not isinstance(state, dict):
        sys.stderr.write("stop-guard.py: state 顶层不是对象\n")
        return 0

    status = state.get("status", "unknown")

    # ========== 2. 已完成则放行 ==========
    if status == "completed":
        return 0

    # ========== 3. 已 blocked 则要求先上报 ==========
    if status == "blocked":
        blocked_reason = state.get("block_reason") or state.get("blocked_reason") or "unknown"
        block(
            f"State is BLOCKED: {blocked_reason}. "
            f"You MUST question() the user before continuing. "
            f"Do not auto-fix blocked tasks."
        )
        return 0

    # ========== 4. 未完成则强制继续 ==========
    tasks = state.get("tasks") or {}
    if not isinstance(tasks, dict):
        # 可能是旧 schema（phase-based），读取 phase 字段判断
        next_phase = state.get("next_phase")
        if next_phase and next_phase != "none":
            block(
                f"Module not completed (phase-based). Next phase: {next_phase}. "
                f"Check state.json and continue the coding loop. Do not claim completion."
            )
        return 0

    remaining = tasks.get("remaining") or []
    current = tasks.get("current") or "none"
    completed = tasks.get("completed") or []
    remaining_count = len(remaining) if isinstance(remaining, list) else 0
    completed_count = len(completed) if isinstance(completed, list) else 0

    still_working = remaining_count > 0 or (current not in ("none", "null", None, ""))

    if still_working:
        block(
            f"Module not completed. {completed_count} tasks done, {remaining_count} remaining. "
            f"Current task: {current}. Check state.json and continue the coding loop. "
            f"Do not claim completion."
        )
        return 0

    # ========== 5. workflow 必须处于 completed 状态才能放行 ==========
    wf = state.get("workflow")
    status = state.get("status", "running")
    if isinstance(wf, dict):
        phase = wf.get("phase", "coding")
        if status != "completed" or phase != "completing":
            block(
                f"Workflow not completed. status={status}, phase={phase}, "
                f"current_plan={wf.get('current_plan')}, current_task={wf.get('current_task')}. "
                f"Do not claim completion until full_reviewing passes."
            )
            return 0

    # 全部通过 — 放行（此时 status 应为 "completed"）
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except Exception as e:
        sys.stderr.write(f"stop-guard.py: 未预期异常: {e}\n")
        # hook 异常不应阻断 Claude — 放行
        sys.exit(0)
