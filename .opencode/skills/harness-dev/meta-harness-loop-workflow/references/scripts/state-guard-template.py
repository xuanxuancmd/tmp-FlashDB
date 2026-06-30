#!/usr/bin/env python3
"""
state-guard.py — Harness Workflow State Validator

统一平铺 schema，不区分主 Agent / executor。
用 stage 字段唯一定位状态。

Exit codes: 0=pass, 1=schema/IO error, 2=blocked
Usage: python state-guard.py <state-json-path>
"""
import json, os, shutil, sys

VALID_STAGES = (
    "coding", "reviewing", "reviewed",
    "evaluating", "evaluated",
    "fixing", "stage_completed", "completed", "blocked",
)
VALID_TRIGGER_STAGES = ("reviewing", "evaluating")


def atomic_write_json(path, data):
    tmp = path + ".tmp"
    try:
        with open(tmp, "w", encoding="utf-8", newline="\n") as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
            f.write("\n"); f.flush(); os.fsync(f.fileno())
        shutil.move(tmp, path)
    finally:
        if os.path.exists(tmp):
            try: os.remove(tmp)
            except OSError: pass


def err(msg):
    print(msg, file=sys.stderr)


def validate_fixing(fixing):
    if not isinstance(fixing, dict):
        return "fixing must be an object"
    ts = fixing.get("trigger_stage")
    if ts not in VALID_TRIGGER_STAGES:
        return f"fixing.trigger_stage invalid: {ts} (expected reviewing|evaluating)"
    return None


def validate(state, path):
    # 必需字段
    for f in ("truth_source", "stage"):
        if f not in state:
            err(f"[state-guard] Missing field: {f}"); return 1

    stage = state["stage"]
    if stage not in VALID_STAGES:
        err(f"[state-guard] Invalid stage: {stage} (expected {VALID_STAGES})"); return 1

    # stage=fixing 时验证 fixing 字段
    if stage == "fixing":
        fixing = state.get("fixing")
        if fixing is None:
            err("[state-guard] stage=fixing requires non-null fixing field"); return 1
        fix_err = validate_fixing(fixing)
        if fix_err:
            err(f"[state-guard] fixing: {fix_err}"); return 1

    # stage=blocked 时验证 blocked_reason
    if stage == "blocked":
        if not state.get("blocked_reason"):
            err("[state-guard] stage=blocked requires blocked_reason"); return 1

    # stage=coding/fixing 时验证 task 字段
    if stage in ("coding", "fixing"):
        if "tasks_completed" not in state or "tasks_remaining" not in state:
            err(f"[state-guard] stage={stage} requires tasks_completed and tasks_remaining"); return 1
        if not isinstance(state["tasks_completed"], list):
            err("[state-guard] tasks_completed must be a list"); return 1
        if not isinstance(state["tasks_remaining"], list):
            err("[state-guard] tasks_remaining must be a list"); return 1

    print(f"[state-guard] OK: stage={stage}, truth_source={state.get('truth_source')}")
    return 0


def main():
    if len(sys.argv) < 2:
        err("[state-guard] Usage: python state-guard.py <state-json-path>"); return 1
    path = sys.argv[1]
    if not os.path.isfile(path):
        err(f"[state-guard] File not found: {path}"); return 1
    try:
        with open(path, "r", encoding="utf-8-sig") as f:
            state = json.load(f)
    except (OSError, json.JSONDecodeError) as exc:
        err(f"[state-guard] Read/parse error: {exc}"); return 1
    if not isinstance(state, dict):
        err("[state-guard] Root must be an object"); return 1
    return validate(state, path)


if __name__ == "__main__":
    sys.exit(main())
