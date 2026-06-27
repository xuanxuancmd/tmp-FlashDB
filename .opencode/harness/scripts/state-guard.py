#!/usr/bin/env python3
"""
state-guard.py — Harness Workflow State Validator (Unified Schema)

One unified JSON schema for both execution state and orchestration state.
Detection: plan_status == null → execution; plan_status != null → orchestration.

Exit codes: 0=pass, 1=schema/IO error, 2=attempt limit reached (BLOCKED)
Usage: python state-guard.py <state-json-path>
"""
import json, os, shutil, sys

MAX_ATTEMPTS = 3
VALID_STATUSES = ("running", "blocked", "completed")
VALID_PHASES = ("coding", "evaluating", "incremental_reviewing", "full_reviewing", "fixing", "completing")
VALID_TRIGGER_STAGES = ("evaluating", "incremental_reviewing", "full_reviewing")
PLAN_STATUSES = ("pending", "running", "completed", "blocked")


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


def block(state, path, scope, count, sig, tried):
    prev = state["status"]
    state["status"] = "blocked"
    state["blocked_reason"] = {
        "type": "attempt_limit", "phase": state["workflow"]["phase"],
        "scope": scope, "attempts": count, "max_attempts": MAX_ATTEMPTS,
        "error_signature": sig, "strategies_tried": tried,
    }
    atomic_write_json(path, state)
    err(f"[state-guard] BLOCKED: scope '{scope}' attempts={count} >= {MAX_ATTEMPTS}")
    print(f"[state-guard] Blocked: '{prev}' -> 'blocked'")


def validate_fixing(fixing):
    if not isinstance(fixing, dict):
        return "fixing must be an object"
    ts = fixing.get("trigger_stage")
    if ts not in VALID_TRIGGER_STAGES:
        return f"fixing.trigger_stage invalid: {ts} (expected evaluating|reviewing)"
    reports = fixing.get("reports")
    if not isinstance(reports, list):
        return "fixing.reports must be a list"
    for i, r in enumerate(reports):
        if not isinstance(r, dict):
            return f"fixing.reports[{i}] must be an object"
        if "path" not in r:
            return f"fixing.reports[{i}].path missing"
    return None


def validate_phase_rules(wf):
    phase = wf.get("phase")
    current_task = wf.get("current_task")
    current_skill = wf.get("current_skill")
    fixing = wf.get("fixing")

    if phase == "coding" and current_task is None:
        return "phase=coding requires non-null current_task"
    if phase == "fixing":
        if current_task is not None:
            return "phase=fixing requires current_task=null"
        if current_skill is not None:
            return "phase=fixing requires current_skill=null"
        fix_err = validate_fixing(fixing)
        if fix_err:
            return f"fixing: {fix_err}"
    if phase == "completing":
        if current_task is not None:
            return "phase=completing requires current_task=null"
        if current_skill is not None:
            return "phase=completing requires current_skill=null"
    return None


def validate_plan_status(plan_status, truth):
    if not isinstance(plan_status, dict) or not plan_status:
        return "plan_status must be non-empty object"
    if set(plan_status.keys()) != set(truth):
        return f"plan_status keys != truth_source_path"
    for k, v in plan_status.items():
        if v not in PLAN_STATUSES:
            return f"plan_status[{k}] invalid: {v} (expected {PLAN_STATUSES})"
    return None


def validate_main(state, path):
    # Top-level required fields
    for f in ("module", "truth_source_path", "status", "workflow"):
        if f not in state:
            err(f"[state-guard] Missing top-level: {f}"); return 1

    truth = state["truth_source_path"]
    if not isinstance(truth, list):
        err("[state-guard] truth_source_path must be a list"); return 1
    if state["status"] not in VALID_STATUSES:
        err(f"[state-guard] status invalid: {state['status']}"); return 1

    wf = state["workflow"]
    if not isinstance(wf, dict):
        err("[state-guard] workflow must be an object"); return 1
    wf_req = ("phase", "current_plan", "current_task", "current_skill",
              "fixing", "tasks_completed", "tasks_remaining",
              "plan_status", "attempt_counts")
    for f in wf_req:
        if f not in wf:
            err(f"[state-guard] Missing workflow field: {f}"); return 1

    plan_status = wf["plan_status"]
    is_orch = plan_status is not None

    for fld in ("tasks_completed", "tasks_remaining"):
        if not isinstance(wf[fld], list):
            err(f"[state-guard] workflow.{fld} must be a list"); return 1
    if not isinstance(wf["attempt_counts"], dict):
        err("[state-guard] attempt_counts must be an object"); return 1

    phase = wf["phase"]
    if phase not in VALID_PHASES:
        err(f"[state-guard] Invalid phase: {phase}"); return 1

    # Orchestration: validate plan_status
    if is_orch:
        ps_err = validate_plan_status(plan_status, truth)
        if ps_err:
            err(f"[state-guard] {ps_err}"); return 1

    # Execution: requires non-empty 'plan' field at top level
    if not is_orch:
        plan = state.get("plan")
        if not isinstance(plan, str) or not plan:
            err("[state-guard] execution state requires non-empty 'plan' field"); return 1

    # Phase rules
    rule_err = validate_phase_rules(wf)
    if rule_err:
        err(f"[state-guard] Phase rule: {rule_err}"); return 1

    # Attempt limit check: scope = plan field (exec) or "global" (orch)
    scope_key = state.get("plan") or "global"
    entry = wf["attempt_counts"].get(scope_key, {})
    if isinstance(entry, dict):
        count = entry.get("count", 0)
        if not isinstance(count, int):
            err(f"[state-guard] attempt_counts[{scope_key}].count not int"); return 1
        if count >= MAX_ATTEMPTS:
            block(state, path, scope_key, count,
                  entry.get("error_signature"), entry.get("strategies_tried", []))
            return 2

    schema_type = "orchestration" if is_orch else "execution"
    print(f"[state-guard] {schema_type}: phase={phase}, plan={state.get('plan')}")
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
    return validate_main(state, path)


if __name__ == "__main__":
    sys.exit(main())
