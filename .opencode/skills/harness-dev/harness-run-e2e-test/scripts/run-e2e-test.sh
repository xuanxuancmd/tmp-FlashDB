#!/usr/bin/env bash
# Licensed to the Apache Software Foundation (ASF) under one or more
# contributor license agreements.  See the NOTICE file distributed with
# this work for additional information regarding copyright ownership.
# The ASF licenses this file to You under the Apache License, Version 2.0
# (the "License"); you may not use this file except in compliance with
# the License.  You may obtain a copy of the License at
#
#    http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# =============================================================================
# run-e2e.sh — Bash E2E test executor for declarative YAML-driven REST API testing.
#
# Behavior mirrors run-e2e.ps1 (PowerShell) for Bash/WSL/Linux environments.
#
# Usage:
#   bash run-e2e.sh -y <yaml-spec-path> -e <evidence-dir> [-m <module-name>]
#   (If -m is omitted, module is inferred from yaml filename: mirror-e2e-test.yaml → mirror)
#
# Exit codes: 0 = all PASS or SKIPPED, 1 = any FAIL
# =============================================================================

set -euo pipefail

SCRIPT_VERSION="1.0.0"

# ─── Globals ─────────────────────────────────────────────────────────────────
YAML_SPEC_PATH=""
MODULE_NAME=""
EVIDENCE_DIR=""
SPEC_JSON=""          # YAML converted to JSON (temp file)
SERVICE_PIDS=()      # Array of background service PIDs (supports multiple processes)
SERVICE_LOG_FILE=""   # Temp file for service stdout/stderr
SERVICE_LOG_DIR=""    # Optional: directory to write service process stdout/stderr log files
declare -A SERVICE_LOG_FILES  # Associative array: PID -> log file path
OVERALL_RESULT="PASS"
SKIPPED_REASON=""
DEPENDENCIES_AVAILABLE=true
BUILD_SUCCESS=false
STARTUP_SUCCESS=false
STARTUP_DURATION_SECONDS=0
LOG_EXPECTATIONS_MET=""
PROCESS_STOPPED=false
FAILED_TEST_NAMES=""  # Comma-separated list of failed test names (for depends_on)
BASE_URL=""           # Base URL from YAML spec for URL substitution

# ─── seq_id auto-increment counter ───────────────────────────────────────────
# Set via CLI parameter -n (default 0). Every command (build, startup,
# when.commands, URLs, cleanup) is substituted with the current SEQ_ID value,
# then the counter increments.
SEQ_ID="0"

# ─── Fail-fast circuit breaker ──────────────────────────────────────────────
# When FAIL_FAST_ENABLED=true (default) and any test FAILs, the flag FAIL_FAST_TRIGGERED
# is set, causing all subsequent scenarios to be SKIPped regardless of depends_on.
# YAML can disable via `fail_fast: false`.
FAIL_FAST_ENABLED="true"
FAIL_FAST_TRIGGERED="false"
FAIL_FAST_TRIGGERING_TEST=""

# ─── Cleanup tracking ───────────────────────────────────────────────────────
DEPENDENCIES_CHECKED=""  # Comma-separated dependency names from YAML

# ─── Temp files (cleaned up on EXIT) ────────────────────────────────────────
TEMP_FILES=()

# ─── Helper: create temp file, register for cleanup ─────────────────────────
make_temp_file() {
    local tf
    tf="$(mktemp)"
    TEMP_FILES+=("$tf")
    echo "$tf"
}

# ─── Helper: JSON-safe string ────────────────────────────────────────────────
json_escape() {
    local s="$1"
    # Escape backslash, double-quote, and control chars
    s="${s//\\/\\\\}"
    s="${s//\"/\\\"}"
    s="${s//$'\n'/\\n}"
    s="${s//$'\r'/\\r}"
    s="${s//$'\t'/\\t}"
    echo "$s"
}

# ─── Helper: get current ISO-8601 timestamp ─────────────────────────────────
iso_timestamp() {
    date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || python3 -c "import datetime;print(datetime.datetime.utcnow().strftime('%Y-%m-%dT%H:%M:%SZ'))"
}

# ─── Helper: parse JSON field (uses jq if available, else python3) ──────────
json_field() {
    local json_data="$1"
    local field_path="$2"
    if command -v jq >/dev/null 2>&1; then
        jq -r "$field_path" <<< "$json_data" 2>/dev/null || echo ""
    else
        python3 -c "
import sys, json
data = json.loads(sys.argv[1])
parts = sys.argv[2].lstrip('.').split('.')
val = data
for p in parts:
    if isinstance(val, dict) and p in val:
        val = val[p]
    elif isinstance(val, list) and p.isdigit():
        val = val[int(p)]
    else:
        val = None
        break
if val is None:
    print('')
elif isinstance(val, bool):
    print('true' if val else 'false')
elif isinstance(val, list):
    print(json.dumps(val))
elif isinstance(val, dict):
    print(json.dumps(val))
else:
    print(str(val))
" "$json_data" "$field_path" 2>/dev/null || echo ""
    fi
}

# ─── Helper: parse JSON array length ────────────────────────────────────────
json_array_len() {
    local json_data="$1"
    local field_path="$2"
    if command -v jq >/dev/null 2>&1; then
        jq -r "$field_path | length" <<< "$json_data" 2>/dev/null || echo "0"
    else
        python3 -c "
import sys, json
data = json.loads(sys.argv[1])
parts = sys.argv[2].lstrip('.').split('.')
val = data
for p in parts:
    if isinstance(val, dict) and p in val:
        val = val[p]
    elif isinstance(val, list) and p.isdigit():
        val = val[int(p)]
    else:
        val = []
        break
print(len(val) if isinstance(val, (list, dict)) else 0)
" "$json_data" "$field_path" 2>/dev/null || echo "0"
    fi
}

# ─── Helper: extract JSON array element as raw JSON ──────────────────────────
json_element() {
    local json_data="$1"
    local field_path="$2"
    local index="$3"
    if command -v jq >/dev/null 2>&1; then
        jq -c ".${field_path}[$index]" <<< "$json_data" 2>/dev/null || echo "{}"
    else
        python3 -c "
import sys, json
data = json.loads(sys.argv[1])
parts = sys.argv[2].split('.')
val = data
for p in parts:
    if isinstance(val, dict) and p in val:
        val = val[p]
    elif isinstance(val, list) and p.isdigit():
        val = val[int(p)]
    else:
        val = []
        break
idx = int(sys.argv[3])
if isinstance(val, list) and idx < len(val):
    print(json.dumps(val[idx]))
else:
    print('{}')
" "$json_data" "$field_path" "$index" 2>/dev/null || echo "{}"
    fi
}

# ─── Helper: check if JSON field is an array ────────────────────────────────
json_is_array() {
    local json_data="$1"
    local field_path="$2"
    if command -v jq >/dev/null 2>&1; then
        local type_str
        type_str="$(jq -r "$field_path | type" <<< "$json_data" 2>/dev/null || echo "null")"
        if [[ "$type_str" == "array" ]]; then
            echo "true"
        else
            echo "false"
        fi
    else
        python3 -c "
import sys, json
data = json.loads(sys.argv[1])
parts = sys.argv[2].lstrip('.').split('.')
val = data
for p in parts:
    if isinstance(val, dict) and p in val:
        val = val[p]
    elif isinstance(val, list) and p.isdigit():
        val = val[int(p)]
    else:
        val = None
        break
print('true' if isinstance(val, list) else 'false')
" "$json_data" "$field_path" 2>/dev/null || echo "false"
    fi
}

 # ─── Argument parsing ────────────────────────────────────────────────────────
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -y)
                YAML_SPEC_PATH="$2"
                shift 2
                ;;
            -m)
                MODULE_NAME="$2"
                shift 2
                ;;
            -e)
                EVIDENCE_DIR="$2"
                shift 2
                ;;
            -s)
                SERVICE_LOG_DIR="$2"
                shift 2
                ;;
            -n)
                SEQ_ID="$2"
                shift 2
                ;;
            *)
                echo "Unknown argument: $1"
                echo "Usage: bash run-e2e.sh -y <yaml-spec-path> -e <evidence-dir> [-m <module-name>] [-s <service-log-dir>] [-n <seq-id>]"
                exit 1
                ;;
        esac
    done

    if [[ -z "$YAML_SPEC_PATH" ]]; then
        echo "ERROR: -y (YAML spec path) is required"
        exit 1
    fi
    if [[ -z "$MODULE_NAME" ]]; then
        # Auto-infer module name from the YAML filename by stripping common suffixes:
        #   mirror-e2e-test.yaml → mirror
        #   runtime-test.yaml    → runtime
        #   connectors.yaml      → connectors
        local yaml_basename
        yaml_basename="$(basename "$YAML_SPEC_PATH")"
        # Strip any recognized extension (.yaml, .yml)
        yaml_basename="${yaml_basename%.yaml}"
        yaml_basename="${yaml_basename%.yml}"
        local inferred="$yaml_basename"
        # Strip longest matching suffix among: -e2e-test, -e2e_test, _e2e_test, -e2e, -test, _test
        if [[ "$inferred" == *"-e2e-test" ]]; then
            inferred="${inferred%-e2e-test}"
        elif [[ "$inferred" == *"-e2e_test" ]]; then
            inferred="${inferred%-e2e_test}"
        elif [[ "$inferred" == *"_e2e_test" ]]; then
            inferred="${inferred%_e2e_test}"
        elif [[ "$inferred" == *"-e2e" ]]; then
            inferred="${inferred%-e2e}"
        elif [[ "$inferred" == *"-test" ]]; then
            inferred="${inferred%-test}"
        elif [[ "$inferred" == *"_test" ]]; then
            inferred="${inferred%_test}"
        fi
        if [[ -z "$inferred" ]]; then
            echo "ERROR: Cannot infer module name from YAML file: $YAML_SPEC_PATH. Please pass -m explicitly."
            exit 1
        fi
        MODULE_NAME="$inferred"
        echo "Module name not provided; inferred from YAML filename: '$MODULE_NAME'"
    fi
    if [[ -z "$EVIDENCE_DIR" ]]; then
        echo "ERROR: -e (evidence dir) is required"
        exit 1
    fi

    if [[ ! -f "$YAML_SPEC_PATH" ]]; then
        echo "ERROR: YAML spec file not found: $YAML_SPEC_PATH"
        exit 1
    fi

    # Create evidence directory if it doesn't exist
    mkdir -p "$EVIDENCE_DIR"

    # Create service log directory if provided
    if [[ -n "$SERVICE_LOG_DIR" ]]; then
        mkdir -p "$SERVICE_LOG_DIR"
        echo "Service log directory: $SERVICE_LOG_DIR"
    fi
}

# ─── Convert YAML to JSON via Python ─────────────────────────────────────────
yaml_to_json() {
    SPEC_JSON="$(make_temp_file)"
    python3 -c "
import sys, yaml, json
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data = yaml.safe_load(f)
with open(sys.argv[2], 'w', encoding='utf-8') as f:
    json.dump(data, f, ensure_ascii=False)
print('OK')
" "$YAML_SPEC_PATH" "$SPEC_JSON" 2>/dev/null
    if [[ ! -f "$SPEC_JSON" ]] || [[ "$(wc -c < "$SPEC_JSON")" -eq 0 ]]; then
        echo "ERROR: Failed to convert YAML to JSON"
        write_skipped_evidence "YAML parsing failed"
        exit 0
    fi
}

# ─── Write SKIPPED evidence (graceful degradation) ──────────────────────────
write_skipped_evidence() {
    local reason="$1"
    local ts
    ts="$(iso_timestamp)"
    local yaml_file_escaped
    yaml_file_escaped="$(json_escape "$YAML_SPEC_PATH")"
    local reason_escaped
    reason_escaped="$(json_escape "$reason")"

    # Build service_log section (usually empty at this point, but include for schema consistency)
    local service_log_json='{"service_logs": null}'
    if [[ -n "$SERVICE_LOG_DIR" ]]; then
        service_log_json='{"service_logs": {}}'
    fi

    local evidence_json
    evidence_json=$(cat <<JSONEOF
{
  "metadata": {
    "module": "$MODULE_NAME",
    "yaml_file": "$yaml_file_escaped",
    "executed_at": "$ts",
    "environment": "Bash",
    "script_version": "$SCRIPT_VERSION"
  },
  "prerequisite_check": {
    "dependencies_available": false,
    "dependencies_checked": [],
    "build_success": false,
    "startup_success": false,
    "startup_duration_seconds": 0,
    "log_expectations_met": null,
    "priority": "LOW"
  },
  "tests": [],
  "cleanup": {
    "resources_deleted": [],
    "process_stopped": false
  },
  "service_log": $service_log_json,
  "summary": {
    "total_tests": 0,
    "passed": 0,
    "failed": 0,
    "skipped": 0,
    "high_priority_failures": 0,
    "low_priority_failures": 0,
    "blocking": false,
    "overall_result": "SKIPPED",
    "skipped_reason": "$reason_escaped"
  }
}
JSONEOF
)
    local outfile="${EVIDENCE_DIR}/${MODULE_NAME}-e2e-result.json"
    echo "$evidence_json" > "$outfile"
    echo "SKIPPED: $reason"
    echo "Evidence written to: $outfile"
}

# ─── Prerequisite: Dependency availability check ─────────────────────────────
# Reads .dependencies from YAML spec and probes each via TCP.
# Sets DEPENDENCIES_AVAILABLE=false and SKIPPED_REASON on failure.
check_dependencies() {
    local deps_json="$1"
    local dep_count
    dep_count=$(echo "$deps_json" | python3 -c "
import sys, json
try:
    d = json.loads(sys.stdin.read())
except:
    d = {}
deps = d.get('dependencies', [])
print(len(deps))
" 2>/dev/null || echo "0")

    if [[ -z "$dep_count" || "$dep_count" == "0" ]]; then
        echo "No dependencies to check."
        return 0
    fi

    echo "Checking $dep_count dependencies..."
    local i=0
    while [[ $i -lt $dep_count ]]; do
        local name host port
        name=$(echo "$deps_json" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
print(d.get('dependencies', [])[$i].get('name', ''))
" 2>/dev/null || echo "")
        host=$(echo "$deps_json" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
print(d.get('dependencies', [])[$i].get('host', ''))
" 2>/dev/null || echo "")
        port=$(echo "$deps_json" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
print(d.get('dependencies', [])[$i].get('port', ''))
" 2>/dev/null || echo "")

        # Track checked dependency names
        if [[ -n "$name" ]]; then
            if [[ -n "$DEPENDENCIES_CHECKED" ]]; then
                DEPENDENCIES_CHECKED="${DEPENDENCIES_CHECKED},${name}"
            else
                DEPENDENCIES_CHECKED="${name}"
            fi
        fi

        # TCP probe
        if [[ -n "$host" && -n "$port" ]]; then
            if timeout 5 bash -c "echo > /dev/tcp/$host/$port" 2>/dev/null; then
                echo "  Dependency '$name' available at ${host}:${port}"
            else
                echo "  Dependency '$name' UNAVAILABLE at ${host}:${port}"
                DEPENDENCIES_AVAILABLE=false
                SKIPPED_REASON="Dependency '$name' unavailable at ${host}:${port}"
                return 1
            fi
        else
            echo "  Dependency '$name' missing host or port, skipping probe."
        fi
        i=$((i + 1))
    done
    return 0
}

# ─── Prerequisite: Build (runs if build_command is defined in given) ──────────
do_build() {
    local build_command
    build_command="$(json_field "$(cat "$SPEC_JSON")" ".given.build_command")"

    if [[ -z "$build_command" || "$build_command" == "null" ]]; then
        BUILD_SUCCESS=true
        echo "No build_command in given section, skipping build."
        return 0
    fi

    # ── seq_id substitution in build_command ──
    build_command="${build_command//\$\{seq_id\}/$SEQ_ID}"
    SEQ_ID=$((SEQ_ID + 1))

    echo "Running build: $build_command"
    # shellcheck disable=SC2086
    if eval "$build_command" >/dev/null 2>&1; then
        BUILD_SUCCESS=true
        echo "Build succeeded."
        return 0
    else
        BUILD_SUCCESS=false
        echo "Build FAILED."
        return 1
    fi
}

# ─── Helper: launch a single service process, append PID to SERVICE_PIDS ───
# Args: $1 = command string, $2 = work_dir, $3 = fallback_log_file (optional)
#
# If SERVICE_LOG_DIR is set (via -s parameter), stdout and stderr are redirected
# to per-process log files in that directory (named <exe>-<timestamp>.log and
# <exe>-<timestamp>.err.log) and the paths are recorded in the SERVICE_LOG_FILES
# associative array for inclusion in evidence as:
#   SERVICE_LOG_FILES[PID]_STDOUT = stdout path
#   SERVICE_LOG_FILES[PID]_STDERR = stderr path
# Otherwise, $3 is used as the log destination (default /dev/null).
_launch_one_process() {
    local cmd="$1"
    local work_dir="$2"
    local fallback_log="${3:-/dev/null}"

    # Determine log destination
    local stdout_log stderr_log
    local log_file
    if [[ -n "$SERVICE_LOG_DIR" ]]; then
        local timestamp
        timestamp="$(date +%Y%m%d-%H%M%S)"
        # Extract executable name from the first token of the command
        local exe_name
        exe_name="$(basename "${cmd%% *}" 2>/dev/null || echo "service")"
        # Strip known extensions (.exe, .sh, .py, etc.)
        exe_name="${exe_name%.*}"
        stdout_log="$SERVICE_LOG_DIR/${exe_name}-${timestamp}.log"
        stderr_log="$SERVICE_LOG_DIR/${exe_name}-${timestamp}.err.log"
        log_file="$stdout_log"
    else
        stdout_log=""
        stderr_log=""
        log_file="$fallback_log"
    fi

    # Start in background, redirect stdout and stderr to separate files (if SERVICE_LOG_DIR set)
    # or combined to fallback_log (if not set).
    # shellcheck disable=SC2086
    if [[ -n "$SERVICE_LOG_DIR" ]]; then
        (cd "$work_dir" && eval "$cmd" >"$stdout_log" 2>"$stderr_log") &
    else
        (cd "$work_dir" && eval "$cmd" >"$log_file" 2>&1) &
    fi
    local pid=$!

    echo "Service process started with PID: $pid"
    SERVICE_PIDS+=("$pid")

    # Record log file paths if log redirection is enabled
    if [[ -n "$SERVICE_LOG_DIR" ]]; then
        SERVICE_LOG_FILES["${pid}_stdout"]="$stdout_log"
        SERVICE_LOG_FILES["${pid}_stderr"]="$stderr_log"
        echo "  Service output will be logged to:"
        echo "    stdout: $stdout_log"
        echo "    stderr: $stderr_log"
    fi

    # Brief fixed wait to let process spin up
    sleep 2

    if ! kill -0 "$pid" 2>/dev/null; then
        echo "Service process $pid exited immediately. Log output:"
        cat "$stderr_log" 2>/dev/null || cat "$log_file" 2>/dev/null || echo "(no log output)"
        return 1
    fi
    return 0
}

# ─── Helper: stop all service processes (SIGTERM, wait, then SIGKILL) ──────
# Iterates SERVICE_PIDS and stops each one. Idempotent: safe to call multiple times.
stop_all_service_processes() {
    if [[ ${#SERVICE_PIDS[@]} -eq 0 ]]; then
        return 0
    fi

    for pid in "${SERVICE_PIDS[@]}"; do
        echo "Stopping service process (PID: $pid)..."
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "  Process $pid already exited"
            continue
        fi
        kill "$pid" 2>/dev/null || true
        # Wait up to 10 seconds for graceful exit
        local wait_count=0
        while kill -0 "$pid" 2>/dev/null && [[ $wait_count -lt 10 ]]; do
            sleep 1
            wait_count=$((wait_count + 1))
        done
        # Force kill if still running
        if kill -0 "$pid" 2>/dev/null; then
            kill -9 "$pid" 2>/dev/null || true
        fi
        echo "  Process $pid stopped"
    done
}

# ─── Resolve startup_command from JSON (scalar string or array) ─────────────
# Args: $1 = JSON string, $2 = JSON path (e.g. ".startup_command")
# Output: newline-separated list of commands to stdout
_resolve_startup_commands() {
    local json_blob="$1"
    local json_path="$2"

    # First try: read as array
    local arr_len
    arr_len="$(json_array_len "$json_blob" "$json_path")"
    if [[ "$arr_len" -gt 0 ]]; then
        local i=0
        while [[ $i -lt $arr_len ]]; do
            local cmd
            cmd="$(json_element "$json_blob" "$json_path" "$i")"
            if [[ -n "$cmd" && "$cmd" != "null" ]]; then
                echo "$cmd"
            fi
            i=$((i + 1))
        done
        return 0
    fi

    # Fallback: read as scalar string
    local scalar
    scalar="$(json_field "$json_blob" "$json_path")"
    if [[ -n "$scalar" && "$scalar" != "null" ]]; then
        echo "$scalar"
    fi
}

# ─── Prerequisite: Start service (build/start only; scenarios do the checking) ──
start_service() {
    local startup_commands
    startup_commands="$(_resolve_startup_commands "$(cat "$SPEC_JSON")" ".given.startup_command")"

    if [[ -z "$startup_commands" ]]; then
        STARTUP_SUCCESS=true
        echo "No startup command specified, skipping."
        return 0
    fi

    SERVICE_LOG_FILE="$(make_temp_file)"

    local startup_start
    startup_start="$(date +%s)"

    # Always prefer the project root (git root) if available as working dir
    local work_dir="$PWD"
    local proj_root
    proj_root="$(cd "$(dirname "$YAML_SPEC_PATH")" && git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")"
    if [[ -n "$proj_root" && -d "$proj_root" ]]; then
        work_dir="$proj_root"
    fi

    local any_failed=false
    while IFS= read -r startup_command; do
        [[ -z "$startup_command" ]] && continue
        # ── seq_id substitution: replace ${seq_id} with current counter, then increment ──
        startup_command="${startup_command//\$\{seq_id\}/$SEQ_ID}"
        SEQ_ID=$((SEQ_ID + 1))
        echo "Starting service: $startup_command"
        if ! _launch_one_process "$startup_command" "$work_dir" "$SERVICE_LOG_FILE"; then
            any_failed=true
        fi
    done <<< "$startup_commands"

    if [[ "$any_failed" == "true" && ${#SERVICE_PIDS[@]} -eq 0 ]]; then
        local elapsed
        elapsed="$(($(date +%s) - startup_start))"
        STARTUP_SUCCESS=false
        STARTUP_DURATION_SECONDS=$elapsed
        return 1
    fi

    local elapsed
    elapsed="$(($(date +%s) - startup_start))"
    STARTUP_SUCCESS=true
    STARTUP_DURATION_SECONDS=$elapsed
    return 0
}

# ─── Prerequisite: Build step (accepts a given step JSON) ─────────────────────
do_build_step() {
    local step_json="$1"
    local build_command
    build_command="$(json_field "$step_json" ".build_command")"

    if [[ -z "$build_command" || "$build_command" == "null" ]]; then
        BUILD_SUCCESS=true
        echo "No build_command in this given step, skipping build."
        return 0
    fi

    # ── seq_id substitution in build_command ──
    build_command="${build_command//\$\{seq_id\}/$SEQ_ID}"
    SEQ_ID=$((SEQ_ID + 1))

    echo "Running build: $build_command"
    # shellcheck disable=SC2086
    if eval "$build_command" >/dev/null 2>&1; then
        BUILD_SUCCESS=true
        echo "Build succeeded."
        return 0
    else
        BUILD_SUCCESS=false
        echo "Build FAILED."
        return 1
    fi
}

# ─── Helper: HTTP readiness probe ──────────────────────────────────────────────
# Polls the given URL until the service responds with any HTTP status code
# (i.e. is listening). Connection errors (refused, timeout) = not ready yet.
#
# Reads readiness config from SPEC_JSON (global):
#   readiness.url              (optional, falls back to BASE_URL)
#   readiness.timeout_seconds (optional, default 60)
#   readiness.interval_seconds(optional, default 2)
#   skip_ssl_verify           (optional, default false)
#
# Returns 0 (ready) or 1 (timeout)
wait_for_readiness() {
    local spec_data
    spec_data="$(cat "$SPEC_JSON")"

    # Determine probe URL: readiness.url > BASE_URL
    local probe_url
    probe_url="$(json_field "$spec_data" ".readiness.url")"
    if [[ -z "$probe_url" || "$probe_url" == "null" ]]; then
        probe_url="$BASE_URL"
    fi

    if [[ -z "$probe_url" ]]; then
        echo "Readiness probe: no probe URL configured (no readiness.url and no base_url); skipping probe, assuming ready."
        return 0
    fi

    # Determine timeout and interval
    local timeout_seconds interval_seconds
    timeout_seconds="$(json_field "$spec_data" ".readiness.timeout_seconds")"
    interval_seconds="$(json_field "$spec_data" ".readiness.interval_seconds")"
    if [[ -z "$timeout_seconds" || "$timeout_seconds" == "null" ]]; then timeout_seconds=60; fi
    if [[ -z "$interval_seconds" || "$interval_seconds" == "null" ]]; then interval_seconds=2; fi

    # Determine whether to skip SSL verification (matches PS1 behavior)
    local skip_ssl
    skip_ssl="$(json_field "$spec_data" ".skip_ssl_verify")"
    local curl_ssl_flag=""
    if [[ "$skip_ssl" == "true" ]]; then
        curl_ssl_flag="-k"
    fi

    echo "Readiness probe: polling $probe_url (timeout=${timeout_seconds}s, interval=${interval_seconds}s)"

    local elapsed=0
    while [[ $elapsed -lt $timeout_seconds ]]; do
        # curl -o /dev/null -s -w "%{http_code}" returns only the HTTP status code.
        # 000 = connection-level failure (not ready); any other code = service is listening.
        local http_code
        # shellcheck disable=SC2086
        http_code="$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout 3 --max-time 5 $curl_ssl_flag "$probe_url" 2>/dev/null || echo "000")"

        if [[ "$http_code" != "000" ]]; then
            echo "  Service ready: HTTP $http_code after ${elapsed}s"
            return 0
        fi

        # Process might have exited early (crashed before becoming ready)
        for p in "${SERVICE_PIDS[@]}"; do
            if ! kill -0 "$p" 2>/dev/null; then
                echo "  Service process $p exited before becoming ready. Stopping readiness probe."
                return 1
            fi
        done

        sleep "$interval_seconds"
        elapsed=$((elapsed + interval_seconds))
    done

    echo "  Readiness probe TIMEOUT after ${timeout_seconds}s — service not responding at $probe_url"
    return 1
}

# ─── Prerequisite: Start service step (accepts a given step JSON) ──
start_service_step() {
    local step_json="$1"
    local startup_commands
    startup_commands="$(_resolve_startup_commands "$step_json" ".startup_command")"

    if [[ -z "$startup_commands" ]]; then
        STARTUP_SUCCESS=true
        echo "No startup command specified in this step, skipping."
        return 0
    fi

    SERVICE_LOG_FILE="$(make_temp_file)"

    local startup_start
    startup_start="$(date +%s)"

    local work_dir="$PWD"
    local proj_root
    proj_root="$(cd "$(dirname "$YAML_SPEC_PATH")" && git rev-parse --show-toplevel 2>/dev/null || echo "$PWD")"
    if [[ -n "$proj_root" && -d "$proj_root" ]]; then
        work_dir="$proj_root"
    fi

    local any_failed=false
    while IFS= read -r startup_command; do
        [[ -z "$startup_command" ]] && continue
        # ── seq_id substitution: replace ${seq_id} with current counter, then increment ──
        startup_command="${startup_command//\$\{seq_id\}/$SEQ_ID}"
        SEQ_ID=$((SEQ_ID + 1))
        echo "Starting service: $startup_command"
        if ! _launch_one_process "$startup_command" "$work_dir" "$SERVICE_LOG_FILE"; then
            any_failed=true
        fi
    done <<< "$startup_commands"

    if [[ "$any_failed" == "true" && ${#SERVICE_PIDS[@]} -eq 0 ]]; then
        local elapsed
        elapsed="$(($(date +%s) - startup_start))"
        STARTUP_SUCCESS=false
        STARTUP_DURATION_SECONDS=$elapsed
        return 1
    fi

    # Readiness probe: poll the service until it is listening or times out.
    # Handles slow-starting services (e.g. Kafka Connect worker needs ~20s to join group).
    if ! wait_for_readiness; then
        local elapsed
        elapsed="$(($(date +%s) - startup_start))"
        STARTUP_SUCCESS=false
        STARTUP_DURATION_SECONDS=$elapsed
        echo "Startup FAILED: service did not become ready within timeout (${elapsed}s)"
        return 1
    fi

    local elapsed
    elapsed="$(($(date +%s) - startup_start))"
    STARTUP_SUCCESS=true
    STARTUP_DURATION_SECONDS=$elapsed
    echo "Startup successful (${elapsed}s)"
    return 0
}

# ─── Execute a single test ───────────────────────────────────────────────────
execute_test() {
    local test_json="$1"
    local test_index="$2"
local test_name
    local then_count
    local retry_max retry_interval has_retry
    local when_is_array when_count

    test_name="$(json_field "$test_json" ".name")"

    # ── Detect mode: Commands vs REST ──
    local cmd_count=0
    cmd_count="$(json_array_len "$test_json" ".when.commands")"
    local scene_mode="rest"
    if [[ "$cmd_count" -gt 0 ]]; then
        scene_mode="commands"
    fi

    # ── Determine when format (array or single object) ──────────────────
    when_is_array="$(json_is_array "$test_json" ".when")"
    if [[ "$when_is_array" == "true" ]]; then
        when_count="$(json_array_len "$test_json" ".when")"
    else
        # Backward compat: when is a single object
        when_count=1
    fi

    echo ""
    echo "─── Test [$test_index]: $test_name ───"

    if [[ "$scene_mode" == "commands" ]]; then
        echo "  Mode: Commands ($cmd_count commands)"
    else
        echo "  Mode: REST"
    fi

# ── Extract priority ──────────────────────────────────────────────────
    local test_priority
    test_priority="$(json_field "$test_json" ".priority")"
    if [[ -z "$test_priority" || "$test_priority" == "null" ]]; then
        test_priority="LOW"
    fi

    # ── fail_fast circuit breaker ──
    # When fail_fast is enabled and an earlier test has already failed,
    # skip this test unconditionally (even if it has no depends_on). This keeps
    # logs clean and makes root-cause analysis straightforward.
    if [[ "$FAIL_FAST_ENABLED" == "true" && "$FAIL_FAST_TRIGGERED" == "true" ]]; then
        local ff_skip_reason="fail_fast: previous test '$FAIL_FAST_TRIGGERING_TEST' failed, remaining scenarios skipped"
        echo "  SKIP: $ff_skip_reason"
        add_test_result "$scene_mode" "$test_name" "SKIP" "" "" "0" "" "$ff_skip_reason" "$test_priority"
        return 0
    fi

    # ── Extract retry config (shared by both modes) ──────────────────────
    retry_max="$(json_field "$test_json" ".retry.max_retries")"
    retry_interval="$(json_field "$test_json" ".retry.interval_seconds")"
    has_retry="false"
    if [[ -n "$retry_max" && "$retry_max" != "null" && "$retry_max" != "0" ]]; then
        has_retry="true"
    fi
    if [[ -z "$retry_interval" ]]; then retry_interval="2"; fi

    # ── Extract wait_seconds (pre-execution delay for async operations) ──
    wait_seconds="$(json_field "$test_json" ".wait_seconds")"
    if [[ -z "$wait_seconds" || "$wait_seconds" == "null" ]]; then wait_seconds="0"; fi

    if [[ "$scene_mode" == "commands" ]]; then
        # ── Commands mode: execute shell commands and validate ──────────
        then_count="$(json_array_len "$test_json" ".then")"

        # Verify 1:1 count match
        if [[ "$then_count" -ne "$cmd_count" ]]; then
            local test_result="FAIL"
            local error_msg="then count ($then_count) != commands count ($cmd_count)"
            echo "  FAIL: $error_msg"
            FAILED_TEST_NAMES="${FAILED_TEST_NAMES},${test_name}"
            add_test_result "commands" "$test_name" "FAIL" "" "" "0" "$error_msg" "" "$test_priority"
            return 0
        fi

        # ── Pre-execution wait for async operations ──
        if [[ "$wait_seconds" -gt 0 ]]; then
            echo "  Waiting ${wait_seconds}s before execution (async delay)..."
            sleep "$wait_seconds"
        fi

        # Retry setup
        local attempts=1
        if [[ "$has_retry" == "true" ]]; then attempts=$((retry_max + 1)); fi

        local attempt=0
        local test_result="FAIL"
        local error_msg=""

        while [[ $attempt -lt $attempts ]]; do
            if [[ $attempt -gt 0 ]]; then
                echo "  Retry $attempt/$retry_max, waiting ${retry_interval}s..."
                sleep "$retry_interval"
            fi

            local cmd_idx=0
            local all_cmd_pass="true"
            error_msg=""

            while [[ $cmd_idx -lt $cmd_count ]]; do
                local cmd
                cmd="$(json_element "$test_json" ".when.commands" "$cmd_idx")"
                cmd="${cmd#\"}"
                cmd="${cmd%\"}"

                # ── seq_id substitution in commands ──
                cmd="${cmd//\$\{seq_id\}/$SEQ_ID}"
                SEQ_ID=$((SEQ_ID + 1))

                echo "  Command[$cmd_idx]: $cmd"

                # Execute command and capture output
                local cmd_output=""
                local cmd_exit_code=0
                cmd_output="$(eval "$cmd" 2>&1)" || cmd_exit_code=$?

                echo "    Exit code: $cmd_exit_code"

                # Get assertion for this command
                local assertion_json
                assertion_json="$(json_element "$test_json" ".then" "$cmd_idx")"

                local assertion_pass="true"

                # Check result_code
                local expected_rc
                expected_rc="$(json_field "$assertion_json" ".result_code")"
                if [[ -n "$expected_rc" && "$expected_rc" != "null" ]]; then
                    local rc_match="false"
                    if [[ "$expected_rc" == "["* ]]; then
                        # Array: OR logic
                        rc_match="$(python3 -c "
import sys, json
actual = int(sys.argv[1])
expected = json.loads(sys.argv[2])
print('true' if actual in expected else 'false')
" "$cmd_exit_code" "$expected_rc" 2>/dev/null || echo "false")"
                    else
                        if [[ "$cmd_exit_code" == "$expected_rc" ]]; then
                            rc_match="true"
                        fi
                    fi

                    if [[ "$rc_match" == "false" ]]; then
                        assertion_pass="false"
                        error_msg="Command[$cmd_idx] result_code: expected $expected_rc, got $cmd_exit_code"
                        echo "    Assertion[$cmd_idx] result_code: ✗ ($error_msg)"
                    else
                        echo "    Assertion[$cmd_idx] result_code: ✓ (exit=$cmd_exit_code)"
                    fi
                fi

                # Check contains (AND logic, case-insensitive)
                if [[ "$assertion_pass" == "true" ]]; then
                    local contains_count
                    contains_count="$(json_array_len "$assertion_json" ".contains")"
                    local ci=0
                    while [[ $ci -lt "$contains_count" ]]; do
                        local expected_sub
                        expected_sub="$(json_element "$assertion_json" ".contains" "$ci")"
                        expected_sub="${expected_sub#\"}"
                        expected_sub="${expected_sub%\"}"

                        if echo "$cmd_output" | grep -qi "$expected_sub" 2>/dev/null; then
                            echo "    Assertion[$cmd_idx] contains '$expected_sub': ✓"
                        else
                            assertion_pass="false"
                            error_msg="Command[$cmd_idx] contains: '$expected_sub' NOT found in stdout"
                            echo "    Assertion[$cmd_idx] contains '$expected_sub': ✗ (NOT found)"
                            break
                        fi
                        ci=$((ci + 1))
                    done
                fi

                # Check not_contains (AND logic, case-insensitive)
                if [[ "$assertion_pass" == "true" ]]; then
                    local nc_count
                    nc_count="$(json_array_len "$assertion_json" ".not_contains")"
                    local ni=0
                    while [[ $ni -lt "$nc_count" ]]; do
                        local not_expected
                        not_expected="$(json_element "$assertion_json" ".not_contains" "$ni")"
                        not_expected="${not_expected#\"}"
                        not_expected="${not_expected%\"}"

                        if echo "$cmd_output" | grep -qi "$not_expected" 2>/dev/null; then
                            assertion_pass="false"
                            error_msg="Command[$cmd_idx] not_contains: '$not_expected' found in stdout (should NOT be)"
                            echo "    Assertion[$cmd_idx] not_contains '$not_expected': ✗ (found)"
                            break
                        else
                            echo "    Assertion[$cmd_idx] not_contains '$not_expected': ✓ (not found)"
                        fi
                        ni=$((ni + 1))
                    done
                fi

                if [[ "$assertion_pass" != "true" ]]; then
                    all_cmd_pass="false"
                    break
                fi

                cmd_idx=$((cmd_idx + 1))
            done

            if [[ "$all_cmd_pass" == "true" ]]; then
                test_result="PASS"
                break
            fi

            attempt=$((attempt + 1))
        done

        # Handle failure
        if [[ "$test_result" == "FAIL" ]]; then
            echo "  Test FAILED: $error_msg"
            FAILED_TEST_NAMES="${FAILED_TEST_NAMES},${test_name}"
        fi

        # Record Commands result
        local duration_ms=0
        add_test_result "commands" "$test_name" "$test_result" "" "" "$duration_ms" "$error_msg" "" "$test_priority"

    else
        # ── REST mode: execute HTTP requests and validate ────────────────
        # ── Extract then assertions ────────────────────────────────────────────
        then_count="$(json_array_len "$test_json" ".then")"

        # ── Pre-execution wait for async operations ──
        if [[ "$wait_seconds" -gt 0 ]]; then
            echo "  Waiting ${wait_seconds}s before execution (async delay)..."
            sleep "$wait_seconds"
        fi

        # ── Execute HTTP request (with retry if configured) ──────────────────
        local attempts=1
        if [[ "$has_retry" == "true" ]]; then
            attempts=$((retry_max + 1))
        fi

    local test_result="FAIL"
    local actual_status=""
    local body_match="false"
    local error_msg=""
    local duration_ms=0
    local response_body=""
    local attempt=0

while [[ $attempt -lt $attempts ]]; do
        if [[ $attempt -gt 0 ]]; then
            echo "  Retry $attempt/$retry_max, waiting ${retry_interval}s..."
            sleep "$retry_interval"
        fi

        # ── Iterate through when steps ──────────────────────────────────
        local when_idx=0
        local total_duration_ms=0

        while [[ $when_idx -lt $when_count ]]; do
            local when_step
            if [[ "$when_is_array" == "true" ]]; then
                when_step="$(json_element "$test_json" ".when" "$when_idx")"
            else
                # Backward compat: when is a single object, extract as-is
                when_step="$(json_field "$test_json" ".when")"
            fi

            local step_method step_url step_body step_headers_json
            step_method="$(json_field "$when_step" ".method")"
            step_url="$(json_field "$when_step" ".url")"

            # Default method
            if [[ -z "$step_method" ]]; then step_method="GET"; fi

            # ── seq_id substitution in REST URL ──
            step_url="${step_url//\$\{seq_id\}/$SEQ_ID}"
            SEQ_ID=$((SEQ_ID + 1))

            echo "  When step [$when_idx]: Method: $step_method, URL: $step_url"

            # ── Extract headers from when_step ────────────────────────────
            step_headers_json="$(json_field "$when_step" ".headers")"
            local curl_headers=""
            if [[ "$step_headers_json" != "" && "$step_headers_json" != "null" ]]; then
                if command -v jq >/dev/null 2>&1; then
                    local header_count
                    header_count="$(jq -r '.headers | length' <<< "$when_step" 2>/dev/null || echo "0")"
                    local h=0
                    while [[ $h -lt $header_count ]]; do
                        local hkey hval
                        hkey="$(jq -r ".headers | keys[$h]" <<< "$when_step" 2>/dev/null)"
                        hval="$(jq -r ".headers | values[$h]" <<< "$when_step" 2>/dev/null)"
                        if [[ -n "$hkey" && -n "$hval" ]]; then
                            curl_headers="${curl_headers} -H \"${hkey}: ${hval}\""
                        fi
                        h=$((h + 1))
                    done
                else
                    # Use python3 to extract headers from when_step
                    curl_headers="$(python3 -c "
import sys, json
data = json.loads(sys.argv[1])
headers = data.get('headers', {})
parts = []
for k, v in headers.items():
    parts.append(f'-H \"{k}: {v}\"')
print(' '.join(parts))
" "$when_step" 2>/dev/null || echo "")"
                fi
            fi

            # ── Extract body from when_step ──────────────────────────────
            step_body="$(json_field "$when_step" ".body")"
            local curl_body_arg=""
            if [[ -n "$step_body" && "$step_body" != "null" ]]; then
                # Body may contain JSON — pass it as-is via -d
                curl_body_arg="-d '${step_body}'"
            fi

            # ── Execute HTTP request for this when step ──────────────────
            local start_ms end_ms
            start_ms="$(date +%s%N 2>/dev/null || date +%s000)"

            local response_tmp http_code_tmp
            response_tmp="$(make_temp_file)"
            http_code_tmp="$(make_temp_file)"

            # shellcheck disable=SC2086
            curl -s -w "\n%{http_code}" -o "$response_tmp" --connect-timeout 10 --max-time 30 -X "$step_method" $curl_headers $curl_body_arg "$step_url" > "$http_code_tmp" 2>/dev/null || true

            end_ms="$(date +%s%N 2>/dev/null || date +%s000)"
            local step_duration_ms=$(( (end_ms - start_ms) / 1000000 ))
            if [[ $step_duration_ms -lt 0 ]]; then step_duration_ms=0; fi
            total_duration_ms=$((total_duration_ms + step_duration_ms))

            # Read results for this step — track as last response
            local http_code
            http_code="$(cat "$http_code_tmp" 2>/dev/null | tail -1)"
            actual_status="$http_code"
            response_body="$(cat "$response_tmp" 2>/dev/null)"

            echo "    HTTP $actual_status (step $when_idx, ${step_duration_ms}ms)"

            when_idx=$((when_idx + 1))
        done

        duration_ms="$total_duration_ms"
        # Ensure non-negative
        if [[ $duration_ms -lt 0 ]]; then duration_ms=0; fi

        echo "  Final response: HTTP $actual_status (attempt $((attempt+1))/$attempts, total ${duration_ms}ms)"

# ── Validate then assertions ──────────────────────────────────────
        local all_assertions_pass="true"
        local assertion_idx=0
        error_msg=""

        while [[ $assertion_idx -lt $then_count ]]; do
            local assertion_json
            assertion_json="$(json_element "$test_json" ".then" "$assertion_idx")"

            # ── Check status ──────────────────────────────────────────────
            local expect_status
            expect_status="$(json_field "$assertion_json" ".status")"
            if [[ -n "$expect_status" && "$expect_status" != "null" ]]; then
                local status_match="false"
                if [[ "$expect_status" == "["* ]]; then
                    # Array of acceptable status codes
                    if command -v jq >/dev/null 2>&1; then
                        local in_array
                        in_array="$(jq -r --arg code "$actual_status" '.status | if type == "array" then map(select(. == ($code | tonumber))) | length > 0 else . == ($code | tonumber) end' <<< "$assertion_json" 2>/dev/null || echo "false")"
                        if [[ "$in_array" == "true" ]]; then status_match="true"; fi
                    else
                        # Use python3 to check if status is in array
                        local py_match
                        py_match="$(python3 -c "
import sys, json
data = json.loads(sys.argv[1])
actual = int(sys.argv[2])
expected = data.get('status', [])
if isinstance(expected, list):
    print('true' if actual in expected else 'false')
else:
    print('true' if actual == expected else 'false')
" "$assertion_json" "$actual_status" 2>/dev/null || echo "false")"
                        if [[ "$py_match" == "true" ]]; then status_match="true"; fi
                    fi
                else
                    # Single expected status code
                    if [[ "$actual_status" == "$expect_status" ]]; then
                        status_match="true"
                    fi
                fi
                if [[ "$status_match" == "false" ]]; then
                    all_assertions_pass="false"
                    if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                    error_msg="${error_msg}Expected status ${expect_status}, got ${actual_status}"
                    echo "    assertion[$assertion_idx] status: ✗ (expected ${expect_status}, got ${actual_status})"
                else
                    echo "    assertion[$assertion_idx] status: ✓ (HTTP ${actual_status})"
                fi
            fi

            # ── Check body (strict JSON map match) ──────────────────────────
            # 'body' expects all key-value pairs from the expected map to be present
            # in the actual top-level JSON response. Extra keys in the actual are OK.
            local body_json
            body_json="$(jq -c '.body // empty' <<< "$assertion_json" 2>/dev/null)"
            if [[ -n "$body_json" && "$body_json" != "null" ]]; then
                local body_match
                body_match="$(python3 -c "
import sys, json
try:
    expected = json.loads(sys.argv[1])
    actual = json.loads(sys.argv[2])
    if not isinstance(expected, dict) or not isinstance(actual, dict):
        print('FAIL:not maps')
        sys.exit(0)
    missing = []
    mismatch = []
    for k, v in expected.items():
        if k not in actual:
            missing.append(k)
        elif str(actual[k]) != str(v):
            mismatch.append((k, str(v), str(actual[k])))
    if missing:
        print('FAIL:missing keys: ' + ', '.join(missing))
    elif mismatch:
        msg = '; '.join(f'{k} expected={e} actual={a}' for k,e,a in mismatch)
        print('FAIL:value mismatch: ' + msg)
    else:
        print('PASS')
except json.JSONDecodeError as e:
    print('FAIL:json parse error: ' + str(e))
except Exception as e:
    print('FAIL:' + str(e))
" "$body_json" "$response_body" 2>/dev/null || echo "FAIL:body match validation error")"
                if [[ "$body_match" == "PASS" ]]; then
                    echo "    assertion[$assertion_idx] body: ✓ (all expected keys match)"
                    body_contains_match="true"
                else
                    echo "    assertion[$assertion_idx] body: ✗ ($body_match)"
                    all_assertions_pass="false"
                    if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                    error_msg="${error_msg}body: $body_match"
                    if [[ "$body_contains_match" != "true" ]]; then
                        body_contains_match="false"
                    fi
                fi
            fi

            # ── Check body_contains ───────────────────────────────────────
            local body_contains_count
            body_contains_count="$(json_array_len "$assertion_json" ".body_contains")"
            if [[ "$body_contains_count" -gt 0 ]]; then
                local bc=0
                while [[ $bc -lt "$body_contains_count" ]]; do
                    local expected_substring
                    expected_substring="$(json_element "$assertion_json" ".body_contains" "$bc")"
                    # Remove surrounding quotes from the JSON value
                    expected_substring="${expected_substring#\"}"
                    expected_substring="${expected_substring%\"}"

                    if echo "$response_body" | grep -qi "$expected_substring" 2>/dev/null; then
                        echo "    assertion[$assertion_idx] body_contains: '$expected_substring' ✓"
                    else
                        echo "    assertion[$assertion_idx] body_contains: '$expected_substring' ✗ (NOT found)"
                        all_assertions_pass="false"
                        if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                        error_msg="${error_msg}body_contains '$expected_substring' NOT found"
                    fi
                    bc=$((bc + 1))
                done
            fi

            # ── Check body_not_contains ───────────────────────────────────
            local body_not_contains_count
            body_not_contains_count="$(json_array_len "$assertion_json" ".body_not_contains")"
            if [[ "$body_not_contains_count" -gt 0 ]]; then
                local bnc=0
                while [[ $bnc -lt "$body_not_contains_count" ]]; do
                    local not_expected_substring
                    not_expected_substring="$(json_element "$assertion_json" ".body_not_contains" "$bnc")"
                    not_expected_substring="${not_expected_substring#\"}"
                    not_expected_substring="${not_expected_substring%\"}"
                    if echo "$response_body" | grep -qi "$not_expected_substring" 2>/dev/null; then
                        echo "    assertion[$assertion_idx] body_not_contains: '$not_expected_substring' ✗ (found but should NOT be)"
                        all_assertions_pass="false"
                        if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                        error_msg="${error_msg}body_not_contains '$not_expected_substring' found (should NOT be)"
                    else
                        echo "    assertion[$assertion_idx] body_not_contains: '$not_expected_substring' ✓ (not found)"
                    fi
                    bnc=$((bnc + 1))
                done
            fi

            # ── Check body_json_path ──────────────────────────────────────
            # Supports two formats:
            #   1. Single path: body_json_path: "$.key" + body_value: "expected"
            #   2. Map format:  body_json_path: { "$.key1": "val1", "$.key2": "val2" }
            local body_json_path_json
            body_json_path_json="$(json_field "$assertion_json" ".body_json_path")"
            if [[ -n "$body_json_path_json" && "$body_json_path_json" != "null" && "$body_json_path_json" != "{}" ]]; then
                # Detect format: string (starts with ") or map (starts with {)
                local jp_is_string="false"
                if [[ "$body_json_path_json" =~ ^\".* ]]; then
                    jp_is_string="true"
                fi

                if [[ "$jp_is_string" == "true" ]]; then
                    # Single path mode: body_json_path is a string, body_value is the expected value
                    # Remove quotes from the path string
                    local single_path="${body_json_path_json#\"}"
                    single_path="${single_path%\"}"
                    local expected_val
                    expected_val="$(json_field "$assertion_json" ".body_value")"
                    expected_val="${expected_val#\"}"
                    expected_val="${expected_val%\"}"

                    if command -v jq >/dev/null 2>&1; then
                        actual_value="$(jq -r "$single_path" <<< "$response_body" 2>/dev/null || echo "")"
                        if [[ "$actual_value" == "$expected_val" ]]; then
                            echo "    assertion[$assertion_idx] body_json_path: '$single_path' = '$actual_value' ✓"
                        else
                            echo "    assertion[$assertion_idx] body_json_path: '$single_path' expected '$expected_val', got '$actual_value' ✗"
                            all_assertions_pass="false"
                            if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                            error_msg="${error_msg}body_json_path '$single_path' expected '$expected_val', got '$actual_value'"
                        fi
                    else
                        local py_jp_result
                        py_jp_result="$(python3 -c "
import sys, json
try:
    resp = json.loads(sys.argv[1])
except:
    resp = {}
path = sys.argv[2]
# Normalize jq-style path: strip leading '$' or '$.' prefix
if path.startswith('$'):
    path = path[2:] if path.startswith('$.') else path[1:]
if path == '':
    actual = json.dumps(resp)
    if actual == sys.argv[3]:
        print('PASS')
    else:
        print(f'FAIL:\$: expected {sys.argv[3]}, got {actual}')
    sys.exit(0)
expected = sys.argv[3]
val = resp
for part in path.split('.'):
    if isinstance(val, dict) and part in val:
        val = val[part]
    else:
        val = None
        break
actual = str(val) if val is not None else ''
if actual == expected:
    print('PASS')
else:
    print(f'FAIL:{path}: expected {expected}, got {actual}')
" "$response_body" "$single_path" "$expected_val" 2>/dev/null || echo "FAIL:body_json_path validation error")"
                        if [[ "$py_jp_result" == "PASS" ]]; then
                            echo "    assertion[$assertion_idx] body_json_path: '$single_path' ✓"
                        else
                            local jp_errors="${py_jp_result#FAIL:}"
                            echo "    assertion[$assertion_idx] body_json_path: ✗ ($jp_errors)"
                            all_assertions_pass="false"
                            if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                            error_msg="${error_msg}body_json_path: $jp_errors"
                        fi
                    fi
                else
                    # Map mode: body_json_path is a map {path: expected, ...}
                    if command -v jq >/dev/null 2>&1; then
                        local path_count
                        path_count="$(jq -r '.body_json_path | length' <<< "$assertion_json" 2>/dev/null || echo "0")"
                        local pi=0
                        while [[ $pi -lt $path_count ]]; do
                            local path_key path_expected actual_value
                            path_key="$(jq -r ".body_json_path | keys[$pi]" <<< "$assertion_json" 2>/dev/null)"
                            path_expected="$(jq -r ".body_json_path | values[$pi]" <<< "$assertion_json" 2>/dev/null)"
                            actual_value="$(jq -r "$path_key" <<< "$response_body" 2>/dev/null || echo "")"
                            if [[ "$actual_value" == "$path_expected" ]]; then
                                echo "    assertion[$assertion_idx] body_json_path: '$path_key' = '$actual_value' ✓"
                            else
                                echo "    assertion[$assertion_idx] body_json_path: '$path_key' expected '$path_expected', got '$actual_value' ✗"
                                all_assertions_pass="false"
                                if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                                error_msg="${error_msg}body_json_path '$path_key' expected '$path_expected', got '$actual_value'"
                            fi
                            pi=$((pi + 1))
                        done
                    else
                        # Use python3 to validate body_json_path
                        local py_jp_result
                        py_jp_result="$(python3 -c "
import sys, json
try:
    resp = json.loads(sys.argv[1])
except:
    resp = {}
paths = json.loads(sys.argv[2])
errors = []
for path, expected in paths.items():
    # Normalize jq-style path: strip leading '$' or '$.' prefix
    original_path = path
    if path.startswith('$'):
        path = path[2:] if path.startswith('$.') else path[1:]
    if path == '':
        actual = json.dumps(resp)
    else:
        val = resp
        for part in path.split('.'):
            if isinstance(val, dict) and part in val:
                val = val[part]
            else:
                val = None
                break
        actual = str(val) if val is not None else ''
    if actual != str(expected):
        errors.append(f'{original_path}: expected {expected}, got {actual}')
if errors:
    print('FAIL:' + '; '.join(errors))
else:
    print('PASS')
" "$response_body" "$body_json_path_json" 2>/dev/null || echo "FAIL:body_json_path validation error")"
                        if [[ "$py_jp_result" == "PASS" ]]; then
                            echo "    assertion[$assertion_idx] body_json_path: ✓"
                        else
                            local jp_errors="${py_jp_result#FAIL:}"
                            echo "    assertion[$assertion_idx] body_json_path: ✗ ($jp_errors)"
                            all_assertions_pass="false"
                            if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                            error_msg="${error_msg}body_json_path: $jp_errors"
                        fi
                    fi
                fi
            fi

            # ── Check body_value (standalone assertion) ─────────────────────
            # Two possible uses of body_value:
            #   1. Used as expected value for body_json_path single-path mode (string) → SKIP here
            #   2. Used as standalone map {field: expected} for top-level field checks → process here
            local body_value_json
            body_value_json="$(json_field "$assertion_json" ".body_value")"
            if [[ -n "$body_value_json" && "$body_value_json" != "null" && "$body_value_json" != "{}" ]]; then
                # Skip if body_value is a scalar (consumed by body_json_path single-path mode)
                if [[ ! "$body_value_json" =~ ^\".*\"$ && ! "$body_value_json" =~ ^-?[0-9] && "$body_value_json" != "true" && "$body_value_json" != "false" ]]; then
                    # It's an object/map → standalone assertion mode
                    if command -v jq >/dev/null 2>&1; then
                    local value_count
                    value_count="$(jq -r '.body_value | length' <<< "$assertion_json" 2>/dev/null || echo "0")"
                    local vi=0
                    while [[ $vi -lt $value_count ]]; do
                        local val_key val_expected actual_val
                        val_key="$(jq -r ".body_value | keys[$vi]" <<< "$assertion_json" 2>/dev/null)"
                        val_expected="$(jq -r ".body_value | values[$vi]" <<< "$assertion_json" 2>/dev/null)"
                        actual_val="$(jq -r ".${val_key}" <<< "$response_body" 2>/dev/null || echo "")"
                        if [[ "$actual_val" == "$val_expected" ]]; then
                            echo "    assertion[$assertion_idx] body_value: '$val_key' = '$actual_val' ✓"
                        else
                            echo "    assertion[$assertion_idx] body_value: '$val_key' expected '$val_expected', got '$actual_val' ✗"
                            all_assertions_pass="false"
                            if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                            error_msg="${error_msg}body_value '$val_key' expected '$val_expected', got '$actual_val'"
                        fi
                        vi=$((vi + 1))
                    done
                else
                    # Use python3 to validate body_value
                    local py_bv_result
                    py_bv_result="$(python3 -c "
import sys, json
try:
    resp = json.loads(sys.argv[1])
except:
    resp = {}
values = json.loads(sys.argv[2])
errors = []
for key, expected in values.items():
    actual = str(resp.get(key, ''))
    if actual != str(expected):
        errors.append(f'{key}: expected {expected}, got {actual}')
if errors:
    print('FAIL:' + '; '.join(errors))
else:
    print('PASS')
" "$response_body" "$body_value_json" 2>/dev/null || echo "FAIL:body_value validation error")"
                    if [[ "$py_bv_result" == "PASS" ]]; then
                        echo "    assertion[$assertion_idx] body_value: ✓"
                    else
                        local bv_errors="${py_bv_result#FAIL:}"
                        echo "    assertion[$assertion_idx] body_value: ✗ ($bv_errors)"
                        all_assertions_pass="false"
                        if [[ -n "$error_msg" ]]; then error_msg="${error_msg}; "; fi
                        error_msg="${error_msg}body_value: $bv_errors"
                    fi
                fi
            fi

            assertion_idx=$((assertion_idx + 1))
        done

        # ── Determine if test passed this attempt ─────────────────────────
        if [[ "$all_assertions_pass" == "true" ]]; then
            test_result="PASS"
            body_match="true"
            echo "  Test PASSED."
            break
        fi

        attempt=$((attempt + 1))
    done

    # ── Handle failure cleanup ────────────────────────────────────────────
    if [[ "$test_result" == "FAIL" ]]; then
        echo "  Test FAILED: $error_msg"
        # Track failed test name for depends_on
        FAILED_TEST_NAMES="${FAILED_TEST_NAMES},${test_name}"

        # cleanup_on_failure not supported - scenarios should handle their own state
    fi

# ── Record test result ────────────────────────────────────────────────
    add_test_result "rest" "$test_name" "$test_result" "$actual_status" "$body_match" "$duration_ms" "$error_msg" "" "$test_priority"
    fi
}

# ─── Accumulate test results into JSON ───────────────────────────────────────
add_test_result() {
    local mode="$1"
    local name="$2"
    local result="$3"
    local status_code="$4"
    local body_contains_match="$5"
    local duration_ms="$6"
    local error_message="$7"
    local skip_reason="$8"
    local priority="$9"

    # Default priority to LOW if not provided
    if [[ -z "$priority" || "$priority" == "null" ]]; then
        priority="LOW"
    fi

    local name_escaped
    name_escaped="$(json_escape "$name")"
    local mode_escaped
    mode_escaped="$(json_escape "$mode")"
    local error_escaped
    error_escaped="$(json_escape "$error_message")"
    local skip_escaped
    skip_escaped="$(json_escape "$skip_reason")"
    local priority_escaped
    priority_escaped="$(json_escape "$priority")"

    # Format status_code as integer or null
    local sc_field
    if [[ -n "$status_code" && "$status_code" != "000" ]]; then
        sc_field="$status_code"
    else
        sc_field="null"
    fi

    # Format body_contains_match as boolean
    local bcm_field
    if [[ "$body_contains_match" == "true" ]]; then
        bcm_field="true"
    elif [[ "$body_contains_match" == "false" ]]; then
        bcm_field="false"
    else
        bcm_field="null"
    fi

    # Format duration_ms as integer
    local dm_field
    if [[ -n "$duration_ms" ]]; then
        dm_field="$duration_ms"
    else
        dm_field="null"
    fi

    local entry
    entry=$(cat <<JSONEOF
{
      "name": "$name_escaped",
      "mode": "$mode_escaped",
      "result": "$result",
      "priority": "$priority_escaped",
      "status_code": $sc_field,
      "body_contains_match": $bcm_field,
      "duration_ms": $dm_field,
      "error_message": "$error_escaped",
      "skip_reason": "$skip_escaped"
    }
JSONEOF
)

    # Append to TEST_RESULTS_JSON array
    if [[ -z "$TEST_RESULTS_JSON" ]]; then
        TEST_RESULTS_JSON="$entry"
    else
        TEST_RESULTS_JSON="${TEST_RESULTS_JSON},
${entry}"
    fi
}

# ─── Run all tests ───────────────────────────────────────────────────────────
run_all_tests() {
    local spec_data
    spec_data="$(cat "$SPEC_JSON")"
    local test_count
    test_count="$(json_array_len "$spec_data" ".scenarios")"

    # Read fail_fast from YAML (default: true). YAML may provide boolean false or string "false".
    local ff_val
    ff_val="$(json_field "$spec_data" ".fail_fast")"
    if [[ "$ff_val" == "false" ]]; then
        FAIL_FAST_ENABLED="false"
    fi
    echo ""
    echo "Running $test_count tests... (fail_fast=$FAIL_FAST_ENABLED)"

    local i=0
    local pre_fail_count=""
    while [[ $i -lt $test_count ]]; do
        local test_json
        test_json="$(json_element "$spec_data" ".scenarios" "$i")"
        # Snapshot failed test count before execution to detect if THIS test fails
        pre_fail_count="$FAILED_TEST_NAMES"

        execute_test "$test_json" "$i"

        # ── fail_fast circuit breaker: arm if this test FAILED ──
        if [[ "$FAIL_FAST_ENABLED" == "true" && "$FAIL_FAST_TRIGGERED" == "false" && "$FAILED_TEST_NAMES" != "$pre_fail_count" ]]; then
            # Extract the name of the test that just failed (last comma-separated token)
            local failed_this_test
            failed_this_test="${FAILED_TEST_NAMES##*,}"
            FAIL_FAST_TRIGGERED="true"
            FAIL_FAST_TRIGGERING_TEST="$failed_this_test"
            echo "  [fail_fast] armed after test '$failed_this_test' failed: remaining scenarios will be skipped"
        fi

        i=$((i + 1))
    done
}

# ─── Cleanup: execute commands and stop process ───────────────────────────────
do_cleanup() {
    echo ""
    echo "─── Cleanup ───"

    # ── Execute cleanup commands from cleanup section ─────────────────────
    local spec_data
    spec_data="$(cat "$SPEC_JSON")"

    # Execute generic shell commands (results not validated)
    local cleanup_cmd_count
    cleanup_cmd_count="$(json_array_len "$spec_data" ".cleanup.commands")"
    if [[ "$cleanup_cmd_count" -gt 0 ]]; then
        local ci=0
        while [[ $ci -lt "$cleanup_cmd_count" ]]; do
            local cmd
            cmd="$(json_element "$spec_data" ".cleanup.commands" "$ci")"
            # Strip surrounding quotes
            cmd="${cmd#\"}"
            cmd="${cmd%\"}"
            if [[ -n "$cmd" ]]; then
                # ── seq_id substitution in cleanup commands ──
                cmd="${cmd//\$\{seq_id\}/$SEQ_ID}"
                SEQ_ID=$((SEQ_ID + 1))
                echo "Cleanup command: $cmd"
                # shellcheck disable=SC2086
                eval "$cmd" >/dev/null 2>&1 || true
            fi
            ci=$((ci + 1))
        done
    fi

    # ── Stop all service processes if requested ───────────────────────────
    local stop_process
    stop_process="$(json_field "$spec_data" ".cleanup.stop_process")"

    if [[ "$stop_process" == "true" ]] && [[ ${#SERVICE_PIDS[@]} -gt 0 ]]; then
        stop_all_service_processes
        PROCESS_STOPPED=true
    elif [[ ${#SERVICE_PIDS[@]} -gt 0 ]]; then
        # Even if stop_process is not explicitly true, clean up our background processes
        for pid in "${SERVICE_PIDS[@]}"; do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
            fi
        done
        PROCESS_STOPPED=true
    fi
}

# ─── Write final evidence JSON ───────────────────────────────────────────────
write_evidence() {
    local ts
    ts="$(iso_timestamp)"
    local yaml_file_escaped
    yaml_file_escaped="$(json_escape "$YAML_SPEC_PATH")"

    # Build dependencies_checked array from comma-separated string
    local dependencies_checked_json="[]"
    if [[ -n "$DEPENDENCIES_CHECKED" ]]; then
        dependencies_checked_json="$(python3 -c "
import sys, json
names = [n.strip() for n in sys.argv[1].split(',') if n.strip()]
print(json.dumps(names))
" "$DEPENDENCIES_CHECKED" 2>/dev/null || echo "[]")"
    fi

    # Determine test summary counts
    local total_tests=0 passed=0 failed=0 skipped=0
    if [[ -n "$TEST_RESULTS_JSON" ]]; then
        total_tests="$(python3 -c "
import sys
lines = sys.argv[1]
# Count entries by counting 'name' keys
count = lines.count('\"name\"')
print(count)
" "$TEST_RESULTS_JSON" 2>/dev/null || echo "0")"
        passed="$(python3 -c "
import sys
lines = sys.argv[1]
print(lines.count('\"result\": \"PASS\"'))
" "$TEST_RESULTS_JSON" 2>/dev/null || echo "0")"
        failed="$(python3 -c "
import sys
lines = sys.argv[1]
print(lines.count('\"result\": \"FAIL\"'))
" "$TEST_RESULTS_JSON" 2>/dev/null || echo "0")"
        skipped="$(python3 -c "
import sys
lines = sys.argv[1]
print(lines.count('\"result\": \"SKIP\"'))
" "$TEST_RESULTS_JSON" 2>/dev/null || echo "0")"
    fi

    # Determine overall result
    if [[ "$OVERALL_RESULT" == "PASS" ]]; then
        if [[ "$failed" -gt 0 ]]; then
            OVERALL_RESULT="FAIL"
        elif [[ "$skipped" -gt 0 && "$passed" -eq 0 ]]; then
            OVERALL_RESULT="SKIPPED"
            if [[ -n "$SKIPPED_REASON" ]]; then
                : # already set
            else
                SKIPPED_REASON="All tests were skipped"
            fi
        fi
    fi

    local overall_escaped
    overall_escaped="$(json_escape "$OVERALL_RESULT")"
    local skipped_reason_escaped
    skipped_reason_escaped="$(json_escape "$SKIPPED_REASON")"

    # log_expectations_met is no longer supported (given-stage validation removed).
    # Always emit `null` to keep the evidence schema stable.
    local lem_field="null"

    # Format dependencies_available as boolean
    local da_field
    if [[ "$DEPENDENCIES_AVAILABLE" == "true" ]]; then
        da_field="true"
    else
        da_field="false"
    fi

# Format build_success as boolean
    local bs_field
    if [[ "$BUILD_SUCCESS" == "true" ]]; then
        bs_field="true"
    else
        bs_field="false"
    fi

    # Format startup_success as boolean
    local ss_field
    if [[ "$STARTUP_SUCCESS" == "true" ]]; then
        ss_field="true"
    else
        ss_field="false"
    fi

    # Determine prerequisite_check priority
    local prereq_priority="LOW"
    if [[ "$BUILD_SUCCESS" == "false" ]]; then prereq_priority="HIGH"; fi
    if [[ "$STARTUP_SUCCESS" == "false" ]]; then prereq_priority="HIGH"; fi

    # Calculate high_priority_failures and low_priority_failures from test results
    local high_pf=0 low_pf=0
    if [[ -n "$TEST_RESULTS_JSON" ]]; then
        high_pf="$(python3 -c "
import sys
lines = sys.argv[1]
# Count entries where result is FAIL and priority is HIGH
count = 0
import re
# Find all test result blocks with FAIL and HIGH priority
pattern = r'\"result\": \"FAIL\"'
prior_pattern = r'\"priority\": \"HIGH\"'
result_matches = [m.start() for m in re.finditer(pattern, lines)]
prior_matches = [m.start() for m in re.finditer(prior_pattern, lines)]
# Simple heuristic: if a FAIL entry also has HIGH priority nearby, count it
for rm in result_matches:
    # Check if there's a HIGH priority within 200 chars of this FAIL
    for pm in prior_matches:
        if abs(rm - pm) < 200:
            count += 1
            break
print(count)
" "$TEST_RESULTS_JSON" 2>/dev/null || echo "0")"
        # low_pf = total failed - high_pf
        low_pf=$(( failed - high_pf ))
        if [[ $low_pf -lt 0 ]]; then low_pf=0; fi
    fi

    # Determine blocking: true if any HIGH priority failure
    local blocking_field="false"
    if [[ "$high_pf" -gt 0 ]] || [[ "$prereq_priority" == "HIGH" ]]; then
        blocking_field="true"
    fi

    # Format process_stopped as boolean
    local ps_field
    if [[ "$PROCESS_STOPPED" == "true" ]]; then
        ps_field="true"
    else
        ps_field="false"
    fi

    # Build tests array JSON
    local tests_json
    if [[ -n "$TEST_RESULTS_JSON" ]]; then
        tests_json="[
    ${TEST_RESULTS_JSON}
  ]"
    else
        tests_json="[]"
    fi

    # Build service_log section JSON
    # If SERVICE_LOG_DIR was provided and SERVICE_LOG_FILES has entries, emit per-PID
    # entries like: { "1234": { "stdout": "...", "stderr": "..." } }
    # Keys in SERVICE_LOG_FILES are "{PID}_stdout" and "{PID}_stderr".
    # If SERVICE_LOG_DIR was provided but empty, emit { "service_logs": {} }
    # Otherwise (not provided), emit { "service_logs": null }
    local service_log_json
    if [[ -n "$SERVICE_LOG_DIR" ]]; then
        if [[ ${#SERVICE_LOG_FILES[@]} -gt 0 ]]; then
            # Collect unique PIDs from keys
            local -A pid_map=()
            for key in "${!SERVICE_LOG_FILES[@]}"; do
                local p="${key%_*}"   # strip _stdout/_stderr suffix
                pid_map["$p"]=1
            done
            local entries=""
            for pid in "${!pid_map[@]}"; do
                local stdout_path="${SERVICE_LOG_FILES[${pid}_stdout]:-}"
                local stderr_path="${SERVICE_LOG_FILES[${pid}_stderr]:-}"
                local stdout_escaped stderr_escaped
                stdout_escaped="$(json_escape "$stdout_path")"
                stderr_escaped="$(json_escape "$stderr_path")"
                if [[ -n "$entries" ]]; then entries="$entries,"; fi
                entries="${entries}\"$pid\": {\"stdout\": \"$stdout_escaped\", \"stderr\": \"$stderr_escaped\"}"
            done
            service_log_json="{\"service_logs\": {$entries}}"
        else
            service_log_json="{\"service_logs\": {}}"
        fi
    else
        service_log_json="{\"service_logs\": null}"
    fi

    local evidence_json
    evidence_json=$(cat <<JSONEOF
{
  "metadata": {
    "module": "$MODULE_NAME",
    "yaml_file": "$yaml_file_escaped",
    "executed_at": "$ts",
    "environment": "Bash",
    "script_version": "$SCRIPT_VERSION"
  },
  "prerequisite_check": {
    "dependencies_available": $da_field,
    "dependencies_checked": $dependencies_checked_json,
    "build_success": $bs_field,
    "startup_success": $ss_field,
    "startup_duration_seconds": $STARTUP_DURATION_SECONDS,
    "log_expectations_met": $lem_field,
    "priority": "$prereq_priority"
  },
  "tests": $tests_json,
  "cleanup": {
    "resources_deleted": [],
    "process_stopped": $ps_field
  },
  "service_log": $service_log_json,
  "summary": {
    "total_tests": $total_tests,
    "passed": $passed,
    "failed": $failed,
    "skipped": $skipped,
    "high_priority_failures": $high_pf,
    "low_priority_failures": $low_pf,
    "blocking": $blocking_field,
    "overall_result": "$overall_escaped",
    "skipped_reason": "$skipped_reason_escaped"
  }
}
JSONEOF
)

    local outfile="${EVIDENCE_DIR}/${MODULE_NAME}-e2e-result.json"
    echo "$evidence_json" > "$outfile"
    echo ""
    echo "Evidence written to: $outfile"
    echo "Overall result: $OVERALL_RESULT"
    echo "Summary: total=$total_tests, passed=$passed, failed=$failed, skipped=$skipped"
}

# ─── Trap: always cleanup on EXIT ────────────────────────────────────────────
on_exit() {
    echo ""
    echo "─── EXIT handler: performing cleanup ───"

    # Kill all service processes if still running
    if [[ ${#SERVICE_PIDS[@]} -gt 0 ]]; then
        for pid in "${SERVICE_PIDS[@]}"; do
            if kill -0 "$pid" 2>/dev/null; then
                echo "Killing service process (PID: $pid)..."
                kill "$pid" 2>/dev/null || true
                sleep 1
                kill -9 "$pid" 2>/dev/null || true
            fi
        done
        PROCESS_STOPPED=true
    fi

    # Execute any remaining cleanup commands (if spec was loaded)
    if [[ -n "$SPEC_JSON" ]] && [[ -f "$SPEC_JSON" ]]; then
        local spec_data
        spec_data="$(cat "$SPEC_JSON" 2>/dev/null || echo "{}")"

        # Execute cleanup commands (generic shell commands)
        local cleanup_cmd_count
        cleanup_cmd_count="$(json_array_len "$spec_data" ".cleanup.commands")"
        local ci=0
        while [[ $ci -lt "$cleanup_cmd_count" ]]; do
            local cmd
            cmd="$(json_element "$spec_data" ".cleanup.commands" "$ci")"
            cmd="${cmd#\"}"
            cmd="${cmd%\"}"
            if [[ -n "$cmd" ]]; then
                echo "  Cleanup command: $cmd"
                # shellcheck disable=SC2086
                eval "$cmd" >/dev/null 2>&1 || true
            fi
            ci=$((ci + 1))
        done
    fi

    # Clean up temp files
    for tf in "${TEMP_FILES[@]}"; do
        rm -f "$tf" 2>/dev/null || true
    done

    echo "EXIT handler complete."
}

trap on_exit EXIT

# ─── Main execution ──────────────────────────────────────────────────────────
main() {
    parse_args "$@"
    yaml_to_json

    # ── Read module from YAML (overrides command-line -Module if present) ──
    local yaml_module
    yaml_module="$(python3 -c "
import sys, json
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data = json.load(f)
if 'module' in data and data['module']:
    print(data['module'])
" "$SPEC_JSON" 2>/dev/null)"
    if [[ -n "$yaml_module" && "$yaml_module" != "$MODULE_NAME" ]]; then
        echo "Module from YAML: '$yaml_module' (overriding '$MODULE_NAME')"
        MODULE_NAME="$yaml_module"
    fi

    # ── Read base_url and apply substitution ─────────────────────────────
    local spec_data
    spec_data="$(cat "$SPEC_JSON")"
    local raw_base_url
    raw_base_url="$(json_field "$spec_data" ".base_url")"
    if [[ -n "$raw_base_url" && "$raw_base_url" != "null" ]]; then
        BASE_URL="$raw_base_url"
        echo "Base URL: $BASE_URL"
        # Substitute ${base_url} in the entire spec JSON
        python3 -c "
import sys, json
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    content = f.read()
content = content.replace('\${base_url}', sys.argv[2])
with open(sys.argv[1], 'w', encoding='utf-8') as f:
    f.write(content)
print('OK')
" "$SPEC_JSON" "$BASE_URL" 2>/dev/null
        spec_data="$(cat "$SPEC_JSON")"
    fi

    echo "seq_id (auto-increment counter, initial): $SEQ_ID"

    echo "╔══════════════════════════════════════════════════════════╗"
    echo "║  E2E Test Executor (Bash) v$SCRIPT_VERSION                ║"
    echo "║  Module: $MODULE_NAME                                    ║"
    echo "║  YAML:   $YAML_SPEC_PATH                                ║"
    echo "╚══════════════════════════════════════════════════════════╝"

    # ── Step 1: Dependency check ─────────────────────────────────────────
    if ! check_dependencies "$spec_data"; then
        OVERALL_RESULT="SKIPPED"
        write_skipped_evidence "$SKIPPED_REASON"
        exit 0
    fi

# ── Step 2 & 3: Process given steps (build + start service) ──────────
    # Re-read spec_data after base_url substitution
    spec_data="$(cat "$SPEC_JSON")"

    # Check if given section exists
    local given_raw
    given_raw="$(json_field "$spec_data" ".given")"

    if [[ -z "$given_raw" || "$given_raw" == "null" || "$given_raw" == "{}" ]]; then
        # No given section at all
        BUILD_SUCCESS=true
        STARTUP_SUCCESS=true
        echo "No given section defined, skipping prerequisites."
    else
        # Determine if given is an array or single object
        local given_is_array
        given_is_array="$(json_is_array "$spec_data" ".given")"

        local given_count=0
        if [[ "$given_is_array" == "true" ]]; then
            given_count="$(json_array_len "$spec_data" ".given")"
        else
            # Backward compat: given is a single object
            given_count=1
        fi

        local given_idx=0
        while [[ $given_idx -lt $given_count ]]; do
            local given_step
            if [[ "$given_is_array" == "true" ]]; then
                given_step="$(json_element "$spec_data" ".given" "$given_idx")"
            else
                # Backward compat: given is a single object, use as step directly
                given_step="$given_raw"
            fi

            # Check for build_command in this step
            local build_cmd
            build_cmd="$(json_field "$given_step" ".build_command")"
            if [[ -n "$build_cmd" && "$build_cmd" != "null" ]]; then
                if ! do_build_step "$given_step"; then
                    SKIPPED_REASON="Build failed"
                    OVERALL_RESULT="SKIPPED"
                    write_skipped_evidence "$SKIPPED_REASON"
                    exit 0
                fi
            fi

            # Check for startup_command in this step
            local startup_cmd
            startup_cmd="$(json_field "$given_step" ".startup_command")"
            if [[ -n "$startup_cmd" && "$startup_cmd" != "null" ]]; then
                if ! start_service_step "$given_step"; then
                    SKIPPED_REASON="Service startup/health-check failed"
                    OVERALL_RESULT="SKIPPED"
                    write_skipped_evidence "$SKIPPED_REASON"
                    exit 0
                fi
            fi

            given_idx=$((given_idx + 1))
        done
    fi

    # ── Step 4: Run tests ──────────────────────────────────────────────────
    run_all_tests

    # ── Step 5: Cleanup ────────────────────────────────────────────────────
    do_cleanup

    # ── Step 6: Write evidence ──────────────────────────────────────────────
    write_evidence

    # ── Exit code ──────────────────────────────────────────────────────────
    if [[ "$OVERALL_RESULT" == "FAIL" ]]; then
        exit 1
    else
        exit 0
    fi
}

main "$@"