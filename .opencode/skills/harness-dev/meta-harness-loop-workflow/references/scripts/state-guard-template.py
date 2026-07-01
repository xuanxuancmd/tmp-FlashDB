#!/usr/bin/env python3
"""
state-guard.py — Harness Workflow State Validator

state.json 的只读校验器（不执行状态转移，只拦截非法写入）。
用 stage 字段唯一定位状态。

VALID_STAGES / VALID_TRIGGER_STAGES 从 workflow.yaml 动态读取（不再硬编码）。
向后兼容：仍接受旧过渡态 reviewed/evaluated（新流程不再产生）。

Exit codes: 0=pass, 1=schema/IO error, 2=blocked
Usage: python state-guard.py <state-json-path>
"""
import json, os, shutil, sys

# ── 名词→动名词映射表（允许硬编码，属命名约定）──────────────────
# yaml stage name（名词）→ state.json stage value（动名词）
NOUN_TO_STATE = {
    "code": "coding",
    "review": "reviewing",
    "evaluate": "evaluating",
    "fix": "fixing",
}

# 固定终态/内置状态（无对应 yaml stage）
FIXED_STATES = ("stage_completed", "completed", "blocked")

# 向后兼容：旧过渡态（新流程不再产生，但断点续传旧 state 可能含）
LEGACY_STATES = ("reviewed", "evaluated")


def parse_simple_yaml(text):
    """最小 YAML 解析器（零外部依赖，仅支持 workflow.yaml 的 schema 子集）。
    支持：注释、key: value、key: 后跟列表（- item）、key: 后跟嵌套对象。"""
    lines = []
    for raw in text.splitlines():
        content = raw
        hash_idx = content.find("#")
        if hash_idx >= 0:
            content = content[:hash_idx]
        indent = len(content) - len(content.lstrip())
        stripped = content.strip()
        if stripped:
            lines.append((indent, stripped))

    pos = 0

    def parse_scalar(val):
        val = val.strip()
        if val.isdigit():
            return int(val)
        if val == "true":
            return True
        if val == "false":
            return False
        return val

    def split_kv(s):
        idx = s.find(":")
        if idx < 0:
            return None
        return s[:idx].strip(), s[idx + 1:].strip()

    def parse_block(parent_indent):
        nonlocal pos
        result = {}
        while pos < len(lines):
            indent, content = lines[pos]
            if indent <= parent_indent:
                break
            kv = split_kv(content)
            if not kv:
                pos += 1
                continue
            key, val = kv
            if val != "":
                result[key] = parse_scalar(val)
                pos += 1
            else:
                cur_indent = indent
                pos += 1
                if pos < len(lines) and lines[pos][0] > cur_indent:
                    if lines[pos][1].startswith("- "):
                        result[key] = parse_list(cur_indent)
                    else:
                        result[key] = parse_block(cur_indent)
                else:
                    result[key] = None
        return result

    def parse_list(parent_indent):
        nonlocal pos
        arr = []
        while pos < len(lines):
            indent, content = lines[pos]
            if indent <= parent_indent:
                break
            if not content.startswith("- "):
                break
            item_content = content[2:]
            item = {}
            kv = split_kv(item_content)
            if kv and kv[1] != "":
                item[kv[0]] = parse_scalar(kv[1])
            pos += 1
            while pos < len(lines):
                sub_indent, sub_content = lines[pos]
                if sub_indent <= indent:
                    break
                if sub_content.startswith("- "):
                    break
                sub_kv = split_kv(sub_content)
                if sub_kv:
                    item[sub_kv[0]] = parse_scalar(sub_kv[1]) if sub_kv[1] != "" else None
                pos += 1
            arr.append(item)
        return arr

    return parse_block(-1)


def load_workflow_config(harness_dir):
    """从 harness_dir/workflow.yaml 加载配置，返回 dict 或 None。"""
    yaml_path = os.path.join(harness_dir, "workflow.yaml")
    if not os.path.isfile(yaml_path):
        return None
    try:
        with open(yaml_path, "r", encoding="utf-8") as f:
            text = f.read()
        return parse_simple_yaml(text)
    except Exception as exc:
        sys.stderr.write(f"[state-guard] workflow.yaml parse error: {exc}\n")
        return None


def derive_valid_stages(config):
    """从 workflow.yaml 推导合法 state 枚举。
    返回 (valid_stages_tuple, valid_trigger_stages_tuple)。"""
    valid = set(FIXED_STATES)
    trigger_stages = set()

    local_stages = config.get("local-stages") or []
    optional_stages = config.get("optional-stages") or {}
    global_stages = config.get("global-stages") or []

    opt_names = set(optional_stages.keys()) if isinstance(optional_stages, dict) else set()

    for stage in local_stages:
        if not isinstance(stage, dict):
            continue
        name = stage.get("name")
        if not name:
            continue
        state_val = NOUN_TO_STATE.get(name, name)
        valid.add(state_val)
        on_failure = stage.get("on_failure")
        if on_failure and on_failure in opt_names:
            trigger_stages.add(state_val)

    for stage in global_stages:
        if not isinstance(stage, dict):
            continue
        name = stage.get("name")
        if not name:
            continue
        # global-stages: state 值 = name 本身（不走 NOUN_TO_STATE 映射）
        state_val = name
        valid.add(state_val)
        on_failure = stage.get("on_failure")
        if on_failure and on_failure in opt_names:
            trigger_stages.add(state_val)

    for opt_name in opt_names:
        state_val = NOUN_TO_STATE.get(opt_name, opt_name)
        valid.add(state_val)

    # 向后兼容
    for s in LEGACY_STATES:
        valid.add(s)

    return tuple(sorted(valid)), tuple(sorted(trigger_stages))


def infer_harness_dir(state_path):
    """state.json 在 {harness}/state/ 下，harness_dir = state.json 所在目录的父目录。"""
    state_dir = os.path.dirname(os.path.abspath(state_path))
    return os.path.dirname(state_dir)


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


def validate_fixing(fixing, valid_trigger_stages):
    if not isinstance(fixing, dict):
        return "fixing must be an object"
    ts = fixing.get("trigger_stage")
    # 向后兼容：接受 reviewed/evaluated（旧过渡态映射回 reviewing/evaluating）
    compat_triggers = set(valid_trigger_stages) | set(LEGACY_STATES)
    if ts not in compat_triggers:
        return f"fixing.trigger_stage invalid: {ts} (expected {sorted(compat_triggers)})"
    return None


def validate(state, path, valid_stages, valid_trigger_stages):
    # 必需字段
    for f in ("truth_source", "stage"):
        if f not in state:
            err(f"[state-guard] Missing field: {f}"); return 1

    stage = state["stage"]
    if stage not in valid_stages:
        err(f"[state-guard] Invalid stage: {stage} (expected {valid_stages})"); return 1

    # stage=fixing 时验证 fixing 字段
    if stage == "fixing":
        fixing = state.get("fixing")
        if fixing is None:
            err("[state-guard] stage=fixing requires non-null fixing field"); return 1
        fix_err = validate_fixing(fixing, valid_trigger_stages)
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

    # 从 workflow.yaml 动态推导合法 stage 枚举
    harness_dir = infer_harness_dir(path)
    config = load_workflow_config(harness_dir)
    if config is None:
        err(f"[state-guard] workflow.yaml not found in {harness_dir}, cannot validate stages"); return 1

    valid_stages, valid_trigger_stages = derive_valid_stages(config)
    return validate(state, path, valid_stages, valid_trigger_stages)


if __name__ == "__main__":
    sys.exit(main())
