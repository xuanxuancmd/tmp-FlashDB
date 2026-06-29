#!/usr/bin/env bun
/**
 * loop-orchestrator.ts — Harness Workflow Autonomous Loop (External Orchestrator)
 *
 * Architecture:
 *   ┌──────────────────────────────────────────────────────────────┐
 *   │  External (this script)        Internal (opencode + skill)   │
 *   │                                coding→evaluating→reviewing   │
 *   │  serve → prompt(prompt) ──────────────────────┐              │
 *   │         ← session.wait() (agent idle)          │              │
 *   │         → read state.json          ←───────────┘              │
 *   │         → decision: completed? blocked? running?              │
 *   │         → if blocked: prompt(recovery_prompt)                 │
 *   │         → loop until terminal state                           │
 *   └──────────────────────────────────────────────────────────────┘
 *
 * The agent loop itself is guaranteed by SessionPrompt.loop() — it NEVER
 * exits on its own mid-task.  This script only runs AFTER the agent loop
 * finishes to decide what to do next.
 *
 * Contract (state.json written by harness-dev-workflow at every phase transition):
 *   {
 *     "module": "...",
 *     "status": "completed" | "blocked" | "running",
 *     "last_run": ISO-8601,
 *     "workflow": {
 *       "phase": "coding|evaluating|coverage_check|incremental_reviewing|
 *                 full_reviewing|fixing|completing",
 *       "current_plan": string | null,
 *       "current_task": string | null,
 *       "current_skill": string | null,
 *       "fixing": null | {trigger_stage, reports},
 *       "tasks_completed": string[],
 *       "tasks_remaining": string[],
 *       "attempt_counts": { [scopeKey]: {count, error_signature, strategies_tried} }
 *     }
 *   }
 *
 * Decision Matrix:
 *   status=completed                → EXIT 0 (success)
 *   status=blocked, retry < limit   → send recovery prompt (new session)
 *   status=blocked, retry >= limit  → EXIT 1 (escalate to human)
 *   status=running                  → send continue prompt (same session)
 *   stale state (no change ×2)      → EXIT 2 (stalled)
 *   single-run timeout              → EXIT 2 (stuck)
 *   max orchestrator rounds         → EXIT 2 (safety cap)
 *
 * Usage:
 *   bun .opencode/harness/scripts/loop-orchestrator.ts --module <name> --plan <path>
 *   bun .opencode/harness/scripts/loop-orchestrator.ts --resume --module <name>
 *   node --experimental-strip-types .opencode/harness/scripts/loop-orchestrator.ts --module <name> --plan <path>
 */

import { spawn, type ChildProcess } from "node:child_process";
import { readFile, readdir, access, mkdir } from "node:fs/promises";
import { join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

// ============================================================================
// Configuration
// ============================================================================

const DEFAULTS = {
  servePort: 4096,
  serveHost: "127.0.0.1",
  serveReadyMs: 8_000,       // wait for serve startup
  runTimeoutMs: 3_600_000,   // per-run timeout (1 hour)
  maxRetries: 3,             // blocked→recovery attempts before escalate
  stateDir: ".opencode/harness/state",
  cli: "opencode",           // CLI binary name
  maxRounds: 15,             // safety cap on total orchestrator rounds
  freshnessLimitMs: 600_000, // 10 min without state.json change → stalled
  pollIntervalMs: 10_000,    // state.json poll interval during run
  roundPauseMs: 3_000,       // pause between orchestrator rounds
} as const;

// ============================================================================
// Types
// ============================================================================

type WorkflowStatus = "running" | "blocked" | "completed";
type WorkflowPhase =
  | "coding" | "evaluating" | "coverage_check"
  | "incremental_reviewing" | "full_reviewing"
  | "fixing" | "completing";

interface AttemptEntry {
  count: number;
  error_signature?: string;
  strategies_tried?: string[];
}

interface BlockedReason {
  type: string;
  phase: string;
  scope: string;
  attempts: number;
  max_attempts: number;
  error_signature?: string;
  strategies_tried?: string[];
}

interface WorkflowState {
  phase: WorkflowPhase;
  current_plan: string | null;
  current_task: string | null;
  current_skill: string | null;
  fixing:
    | null
    | { trigger_stage: string; reports: Array<{ path: string }> };
  tasks_completed: string[];
  tasks_remaining: string[];
  plan_status: Record<string, string> | null;
  attempt_counts: Record<string, AttemptEntry>;
}

interface StateFile {
  module: string;
  plan?: string;
  truth_source_path: string[];
  status: WorkflowStatus;
  last_run: string;
  workflow: WorkflowState;
  blocked_reason?: BlockedReason;
}

interface Decision {
  action: "continue" | "recover" | "escalate" | "complete" | "retry";
  reason: string;
  blockedDetails?: {
    scope: string;
    attempts: number;
    maxAttempts: number;
    strategiesTried: string[];
  };
}

interface LoopState {
  roundNum: number;
  recoveryCount: number;
  stallCount: number;
  startTime: number;
  lastStateSig: string;
  history: Array<{
    round: number;
    action: string;
    reason: string;
    ts: string;
  }>;
}

interface RunResult {
  ok: boolean;
  timedOut: boolean;
  exitType: "completed" | "timeout" | "error";
  outputPreview: string;
}

interface CliArgs {
  module: string;
  plan?: string;
  resume: boolean;
  maxRounds: number;
  maxRetries: number;
  stateDir: string;
  servePort: number;
  runTimeoutMs: number;
  dryRun: boolean;
  cli: string;
}

// ============================================================================
// Prompt Templates
// ============================================================================

const INIT_PROMPT = (module: string, planPath: string) => `\
Execute the harness-dev-workflow skill.

Parameters:
- plan_list = ["${planPath}"]
- module_name = "${module}"

Pre-flight:
Read the state file at:
  .opencode/harness/state/${module}-workflow-state.json

If it exists with status=running:
  Resume from the recorded phase and current_task.
If absent, status=completed, or corrupted:
  Start fresh.

Execution:
  coding(TDD) → evaluating → coverage_check → reviewing → completing

Rules (MANDATORY — violating these is a workflow failure):
1. MUST refresh state.json at EVERY phase transition.
2. NEVER call question() — make all decisions autonomously.
3. If blocked in fixing: attempt with a NEW strategy each time
   (same strategy twice = automatic failure).
4. If 3 fixing attempts fail: set status=blocked in state.json,
   then stop and wait for the orchestrator to decide recovery.
5. Run \`cargo check\` and \`cargo test\` after every task completion.
`;

const CONTINUE_PROMPT = `\
Continue the harness-dev-workflow.

Steps:
1. Read the state file to find current phase and current_task.
2. Resume from EXACTLY that point — do NOT restart completed work.
3. Execute the next task or advance to the next phase.
4. Refresh state.json at every phase transition.
5. When the workflow is fully done, set status=completed in state.json.

Do NOT stop without finishing the current phase or updating state.json.
`;

const RECOVERY_PROMPT = (
  module: string,
  details: { scope: string; attempts: number; maxAttempts: number; strategies: string[] },
) => `\
The workflow is currently BLOCKED.

Blocked details:
  - scope: ${details.scope}
  - attempts so far: ${details.attempts}/${details.maxAttempts}
  - strategies already tried: ${JSON.stringify(details.strategies, null, 2)}

Read the state file:
  .opencode/harness/state/${module}-workflow-state.json

Also read any error reports referenced in the fixing.reports field.

Then:
1. Analyze WHY the previous attempts FAILED (not what went wrong at the surface).
2. Identify the ROOT CAUSE:
   - Is it a type system issue? (wrong trait bound, lifetime mismatch)
   - Is it a logic issue? (wrong algorithm, missing branch)
   - Is it an integration issue? (missing import, wrong module path)
3. Implement a fix using a FUNDAMENTALLY DIFFERENT strategy:
   - If previous attempts touched code structure → try error-handling.
   - If they touched types → try logic.
   - If they touched logic → try architecture.
4. Run the validation chain:
     cargo check && cargo test
5. If validation passes:
   - Reset status to "running" in state.json
   - Continue the workflow from the phase that was blocked.
6. If validation still fails:
   - Set status=blocked with updated blocked_reason
   - Stop and wait for the orchestrator.

CRITICAL: You MUST produce a working fix OR set status=blocked again.
Do NOT exit without updating state.json.
`;

const STALENESS_RECOVERY_PROMPT = `\
The workflow state has not changed for over 10 minutes.
This indicates the agent is stuck in a loop without updating state.

Steps to recover:
1. Read the state file to find the current phase and task.
2. COMPLETE the current phase — finish whatever is in progress.
3. Refresh state.json IMMEDIATELY to confirm you are alive.
4. Advance to the next phase or task.
5. Continue the workflow normally.

DO NOT start from the beginning.
DO NOT stop without updating state.json.
`;

// ============================================================================
// Server Manager
// ============================================================================

class ServerManager {
  private proc: ChildProcess | null = null;
  private port: number;
  private host: string;
  private projectDir: string;
  private cli: string;

  constructor(port: number, host: string, projectDir: string, cli: string) {
    this.port = port;
    this.host = host;
    this.projectDir = projectDir;
    this.cli = cli;
  }

  async start(): Promise<void> {
    this.proc = spawn(this.cli, ["serve", "--port", String(this.port), "--hostname", this.host], {
      cwd: this.projectDir,
      stdio: ["ignore", "pipe", "pipe"],
    });

    this.proc.stdout?.on("data", (chunk: Buffer) => {
      const msg = chunk.toString().trim();
      if (msg) log(`[serve] ${msg}`);
    });

    this.proc.stderr?.on("data", (chunk: Buffer) => {
      const msg = chunk.toString().trim();
      if (msg) log(`[serve:err] ${msg}`);
    });

    // Wait for readiness
    await sleep(DEFAULTS.serveReadyMs);

    if (this.proc.exitCode !== null) {
      throw new Error(`opencode serve exited early with code ${this.proc.exitCode}`);
    }

    // Health probe: poll /event endpoint until responsive
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      try {
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 2000);
        // GET /event returns SSE stream. We only need to confirm the server
        // accepts the connection — any HTTP response (even 4xx) proves it's up.
        const res = await fetch(
          `http://${this.host}:${this.port}/event`,
          { signal: controller.signal },
        );
        clearTimeout(timeout);
        if (res.status > 0) {
          log(`Server reachable at http://${this.host}:${this.port}`);
          return;
        }
      } catch {
        // connection refused or aborted — not ready yet
        await sleep(1000);
      }
    }

    this.stop();
    throw new Error(`opencode serve not reachable after 15s`);
  }

  stop(): void {
    if (this.proc && this.proc.exitCode === null) {
      this.proc.kill("SIGTERM");
      log("Server process terminated.");
    }
  }
}

// ============================================================================
// OpenCode API Client (via HTTP, not SDK — no runtime dependency)
// ============================================================================

class OpencodeClient {
  private baseUrl: string;
  private sessionId: string | null = null;

  constructor(host: string, port: number) {
    this.baseUrl = `http://${host}:${port}`;
  }

  /** Create a new session. Returns session ID.
   *
   *  Sets permission rules that DENY question/plan_enter/plan_exit so the
   *  agent is FORCED to make autonomous decisions instead of asking humans.
   *  This mirrors what `opencode run` (non-interactive) does automatically
   *  in OpenCode's CLI layer (run.ts L413-L431) — but via the HTTP API we
   *  must pass these explicitly.
   */
  async createSession(title?: string): Promise<string> {
    const res = await this.request("/session", {
      method: "POST",
      body: JSON.stringify({
        title: title ?? "harness-loop",
        // CRITICAL: deny question() so the agent never stops to ask humans.
        // Deny plan_enter/plan_exit as these are TUI-only features.
        permission: [
          { permission: "question",    action: "deny", pattern: "*" },
          { permission: "plan_enter",  action: "deny", pattern: "*" },
          { permission: "plan_exit",   action: "deny", pattern: "*" },
        ],
      }),
    });
    const data = (await res.json()) as { id: string };
    if (!data.id) throw new Error("session.create did not return id");
    this.sessionId = data.id;
    return data.id;
  }

  /** Create a new message (prompt) and return immediately (fire-and-forget).
   *  The agent will process it in the background. */
  async promptAsync(prompt: string): Promise<string | null> {
    if (!this.sessionId) throw new Error("No session — call createSession first");

    const res = await this.request(`/session/${this.sessionId}/prompt_async`, {
      method: "POST",
      body: JSON.stringify({
        parts: [{ type: "text", text: prompt }],
      }),
    });

    // prompt_async returns 204 No Content
    if (!res.ok && res.status !== 204) {
      const body = await res.text().catch(() => "");
      throw new Error(`prompt_async failed (${res.status}): ${body}`);
    }

    const text = await res.text().catch(() => "");
    try {
      const data = JSON.parse(text || "{}") as { messageID?: string };
      return data.messageID ?? null;
    } catch {
      return null;
    }
  }

  /** Wait until the session becomes idle.
   *  Returns when the agent loop finishes processing the current prompt. */
  async wait(): Promise<void> {
    if (!this.sessionId) throw new Error("No session — call createSession first");

    const res = await this.request(`/session/${this.sessionId}/wait`, {
      method: "POST",
    });

    if (!res.ok) {
      const body = await res.text().catch(() => "");
      throw new Error(`session.wait failed (${res.status}): ${body}`);
    }
  }

  /** Get the last assistant message text (for diagnostics). */
  async getLatestAssistantMessage(): Promise<string | null> {
    if (!this.sessionId) return null;
    try {
      const res = await this.request(`/session/${this.sessionId}/message`, {
        method: "GET",
      });
      const messages = (await res.json()) as Array<{
        role: string;
        parts?: Array<{ type: string; text?: string }>;
      }>;
      const lastAssistant = [
        ...messages.filter((m) => m.role === "assistant"),
      ].pop();
      if (lastAssistant?.parts) {
        const textPart = lastAssistant.parts.find((p) => p.type === "text");
        return textPart?.text ?? null;
      }
    } catch {
      // Non-critical — just for diagnostics
    }
    return null;
  }

  private async request(path: string, init: RequestInit): Promise<Response> {
    return fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: {
        "content-type": "application/json",
        ...(init.headers ?? {}),
      },
    });
  }

  get currentSessionId(): string | null {
    return this.sessionId;
  }
}

// ============================================================================
// State Manager
// ============================================================================

class StateManager {
  private stateDir: string;
  private module: string;

  constructor(stateDir: string, module: string) {
    this.stateDir = stateDir;
    this.module = module;
  }

  async findStatePath(): Promise<string | null> {
    try {
      // Prefer exact module match first (fast path)
      const exact = join(this.stateDir, `${this.module}-workflow-state.json`);
      try {
        await access(exact);
        return exact;
      } catch {
        // fall through to directory scan
      }

      const entries = (await readdir(this.stateDir))
        .filter((f) => f.endsWith("-workflow-state.json"))
        .sort();

      if (entries.length === 0) return null;

      // Fall back to most recently modified (last alphabetically for same dir)
      return join(this.stateDir, entries[entries.length - 1]);
    } catch {
      return null;
    }
  }

  async read(): Promise<StateFile | null> {
    const path = await this.findStatePath();
    if (!path) return null;
    try {
      const raw = await readFile(path, "utf-8");
      const data = JSON.parse(raw) as StateFile;
      return this.validate(data) ? data : null;
    } catch {
      return null;
    }
  }

  async getSignature(): Promise<string> {
    const state = await this.read();
    if (!state) return "";
    return JSON.stringify(state, Object.keys(state).sort());
  }

  private validate(data: unknown): data is StateFile {
    if (!data || typeof data !== "object") return false;
    const d = data as Record<string, unknown>;
    if (!d.status || typeof d.status !== "string") return false;
    if (!["running", "blocked", "completed"].includes(d.status as string)) return false;
    if (!d.workflow || typeof d.workflow !== "object") return false;
    return true;
  }
}

// ============================================================================
// Decision Engine
// ============================================================================

class DecisionEngine {
  private maxRetries: number;

  constructor(maxRetries: number) {
    this.maxRetries = maxRetries;
  }

  decide(
    state: StateFile | null,
    recoveryCount: number,
    stallCount: number,
  ): Decision {
    // ---- state absent ----
    if (!state) {
      if (recoveryCount > 0) {
        return {
          action: "retry",
          reason:
            "Agent did not update state.json after recovery prompt; " +
            "will resend with more explicit instruction",
        };
      }
      return {
        action: "continue",
        reason: "state.json not found (agent may still be initializing)",
      };
    }

    const { status, workflow, blocked_reason } = state;

    // ---- completed ----
    if (status === "completed") {
      return { action: "complete", reason: "workflow.status = completed" };
    }

    // ---- blocked ----
    if (status === "blocked") {
      const br = blocked_reason ?? ({} as BlockedReason);
      const attempts = br.attempts ?? 0;
      const maxA = br.max_attempts ?? 3;
      const scope = br.scope ?? "unknown";
      const strategies = br.strategies_tried ?? [];

      if (recoveryCount < this.maxRetries) {
        return {
          action: "recover",
          reason: `blocked (${scope}, ${attempts}/${maxA}); recovery ${recoveryCount + 1}/${this.maxRetries}`,
          blockedDetails: {
            scope,
            attempts,
            maxAttempts: maxA,
            strategiesTried: strategies,
          },
        };
      }

      return {
        action: "escalate",
        reason: `blocked × ${this.maxRetries} recovery attempts exhausted; escalate to human`,
        blockedDetails: { scope, attempts, maxAttempts: maxA, strategiesTried: strategies },
      };
    }

    // ---- running ----
    if (status === "running") {
      if (stallCount >= 2) {
        return {
          action: "escalate",
          reason: `status=running but state unchanged for 2+ rounds (agent stuck in infinite loop)`,
        };
      }
      return {
        action: "continue",
        reason: `status=running, phase=${workflow.phase}, task=${workflow.current_task ?? "none"}`,
      };
    }

    // ---- unknown ----
    return {
      action: "retry",
      reason: `unknown status='${status}'; sending safety continue prompt`,
    };
  }
}

// ============================================================================
// Orchestrator
// ============================================================================

class LoopOrchestrator {
  private args: CliArgs;
  private server: ServerManager;
  private client: OpencodeClient;
  private state: StateManager;
  private engine: DecisionEngine;
  private projectDir: string;

  private loop: LoopState = {
    roundNum: 0,
    recoveryCount: 0,
    stallCount: 0,
    startTime: Date.now(),
    lastStateSig: "",
    history: [],
  };

  constructor(args: CliArgs) {
    this.args = args;
    this.projectDir = resolve(".");

    this.server = new ServerManager(
      args.servePort,
      DEFAULTS.serveHost,
      this.projectDir,
      args.cli,
    );
    this.client = new OpencodeClient(DEFAULTS.serveHost, args.servePort);
    this.state = new StateManager(args.stateDir, args.module);
    this.engine = new DecisionEngine(args.maxRetries);
  }

  async execute(): Promise<number> {
    printBanner(this.args);

    // ---- Start server ----
    log(`Starting opencode serve on ${DEFAULTS.serveHost}:${this.args.servePort} ...`);
    await this.server.start();

    // ---- Start monitor (poll state.json during runs) ----
    const monitor = new StateMonitor(
      this.state,
      (sig) => { this.loop.lastStateSig = sig; this.loop.stallCount = 0; },
      () => { this.loop.stallCount++; },
    );
    monitor.start();

    try {
      return await this.runLoop();
    } finally {
      monitor.stop();
      this.server.stop();
    }
  }

  private async runLoop(): Promise<number> {
    let prevWasRecovery = false;

    while (this.loop.roundNum < this.args.maxRounds) {
      this.loop.roundNum++;
      const roundStart = Date.now();

      // ---- Read state ----
      const state = await this.state.read();
      const decision = this.engine.decide(state, this.loop.recoveryCount, this.loop.stallCount);
      this.logRound(decision);

      // ---- Terminal states ----
      if (decision.action === "complete") {
        printSuccess(this.loop);
        return 0;
      }

      if (decision.action === "escalate") {
        printEscalation(this.loop, decision);
        return 1;
      }

      // ---- Dry-run mode ----
      if (this.args.dryRun) {
        const prompt = this.buildPrompt(decision);
        log(`[dry-run] Prompt (${prompt.length} chars):\n${prompt.slice(0, 300)}...`);
        log(`Exiting dry-run.`);
        return 0;
      }

      // ---- Build prompt ----
      const prompt = this.buildPrompt(decision);

      // ---- Execute prompt via opencode API ----
      log(`\n➤ Sending prompt (${prompt.length} chars, recovery=${decision.action === "recover"}) ...`);

      const result = await this.executePrompt(prompt, decision);

      if (!result.ok) {
        log(`⚠ Run ended: ${result.exitType}`);
        if (result.timedOut) {
          log(`Agent exceeded run timeout (${this.args.runTimeoutMs / 1000}s)`);
          return 2;
        }
      }

      // ---- Post-run state check ----
      const postState = await this.state.read();
      if (postState?.status === "completed") {
        printSuccess(this.loop);
        return 0;
      }

      // ---- Track recovery ----
      if (decision.action === "recover") {
        this.loop.recoveryCount++;
        prevWasRecovery = true;
      } else {
        prevWasRecovery = false;
      }

      // ---- Round summary ----
      const elapsed = ((Date.now() - roundStart) / 1000).toFixed(1);
      const total = ((Date.now() - this.loop.startTime) / 1000).toFixed(1);
      log(`  Round ${this.loop.roundNum} done (${elapsed}s). Total: ${total}s`);
      log(`  Recovery: ${this.loop.recoveryCount}/${this.args.maxRetries}  Stall: ${this.loop.stallCount}`);

      // ---- Pause between rounds ----
      await sleep(DEFAULTS.roundPauseMs);
    }

    // Max rounds exceeded
    printMaxRounds(this.loop);
    return 2;
  }

  private buildPrompt(decision: Decision): string {
    const { module, plan } = this.args;

    if (this.loop.roundNum === 1 && decision.action === "continue") {
      return INIT_PROMPT(module, plan!);
    }

    if (decision.action === "recover") {
      const d = decision.blockedDetails!;
      return RECOVERY_PROMPT(module, {
        scope: d.scope,
        attempts: d.attempts,
        maxAttempts: d.maxAttempts,
        strategies: d.strategiesTried,
      });
    }

    if (this.loop.stallCount >= 2) {
      return STALENESS_RECOVERY_PROMPT;
    }

    // General continue: round > 1
    return CONTINUE_PROMPT;
  }

  /**
   * Send prompt to agent and wait for the agent loop to complete.
   * Uses prompt_async + session.wait() from the OpenCode API.
   */
  private async executePrompt(prompt: string, decision: Decision): Promise<RunResult> {
    try {
      // For recovery, create a fresh session (isolated context).
      // For continue, reuse the same session (preserves conversation).
      if (decision.action === "recover") {
        await this.client.createSession(
          `recovery-${this.loop.recoveryCount + 1}`,
        );
      } else if (this.loop.roundNum === 1) {
        await this.client.createSession("harness-init");
      }
      // else: use existing session

      // Fire the prompt asynchronously
      await this.client.promptAsync(prompt);

      log(`  Agent processing (waiting for idle) ...`);

      // Wait for the agent loop to complete, with timeout.
      // The clearTimeout MUST run regardless of which promise wins the race
      // to prevent the timeout timer from leaking after success.
      let timerID: ReturnType<typeof setTimeout> | undefined;
      try {
        await Promise.race([
          this.client.wait(),
          new Promise<void>((_, reject) => {
            timerID = setTimeout(
              () => reject(new Error("timeout")),
              this.args.runTimeoutMs,
            );
          }),
        ]);
      } finally {
        if (timerID !== undefined) clearTimeout(timerID);
      }

      return {
        ok: true,
        timedOut: false,
        exitType: "completed",
        outputPreview: "",
      };
    } catch (err: unknown) {
      const message = (err as Error).message;
      if (message === "timeout") {
        return {
          ok: false,
          timedOut: true,
          exitType: "timeout",
          outputPreview: `Run exceeded ${this.args.runTimeoutMs / 1000}s`,
        };
      }
      return {
        ok: false,
        timedOut: false,
        exitType: "error",
        outputPreview: message,
      };
    }
  }

  private logRound(d: Decision): void {
    const ts = new Date().toISOString().replace("T", " ").slice(0, 19);
    this.loop.history.push({
      round: this.loop.roundNum,
      action: d.action,
      reason: d.reason,
      ts,
    });
    log(`[${ts}] Round ${this.loop.roundNum}: ${d.action.toUpperCase()} — ${d.reason}`);
  }
}

// ============================================================================
// State Monitor (polls state.json during runs for stall detection)
// ============================================================================

class StateMonitor {
  private state: StateManager;
  private onFresh: (sig: string) => void;
  private onStale: () => void;
  private timer: ReturnType<typeof setInterval> | null = null;
  private lastSig: string = "";

  constructor(state: StateManager, onFresh: (sig: string) => void, onStale: () => void) {
    this.state = state;
    this.onFresh = onFresh;
    this.onStale = onStale;
  }

  start(): void {
    this.timer = setInterval(async () => {
      const sig = await this.state.getSignature();
      if (sig === this.lastSig) {
        this.onStale();
      } else {
        this.lastSig = sig;
        this.onFresh(sig);
      }
    }, DEFAULTS.pollIntervalMs);
  }

  stop(): void {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }
}

// ============================================================================
// CLI Argument Parsing
// ============================================================================

function parseArgs(argv: string[]): CliArgs {
  const get = (flag: string): boolean => argv.includes(flag);
  const val = (flag: string): string | undefined => {
    const idx = argv.indexOf(flag);
    if (idx === -1) return undefined;
    return argv[idx + 1];
  };
  const num = (flag: string, def: number): number => {
    const v = val(flag);
    return v !== undefined ? parseInt(v, 10) : def;
  };

  const module = val("--module");
  if (!module) {
    printUsage();
    process.exit(2);
  }

  return {
    module,
    plan: val("--plan"),
    resume: get("--resume"),
    maxRounds: num("--max-rounds", DEFAULTS.maxRounds),
    maxRetries: num("--max-retries", DEFAULTS.maxRetries),
    stateDir: val("--state-dir") ?? DEFAULTS.stateDir,
    servePort: num("--serve-port", DEFAULTS.servePort),
    runTimeoutMs: num("--run-timeout", DEFAULTS.runTimeoutMs / 1000) * 1000,
    dryRun: get("--dry-run"),
    cli: val("--cli") ?? DEFAULTS.cli,
  };
}

// ============================================================================
// Output Formatting
// ============================================================================

function log(msg: string): void {
  process.stdout.write(`  ${msg}\n`);
}

function printBanner(args: CliArgs): void {
  const bar = "=".repeat(70);
  process.stdout.write(`\n${bar}\n`);
  process.stdout.write(`  loop-orchestrator.ts\n`);
  process.stdout.write(`  module=${args.module}  plan=${args.plan ?? "(resume)"}\n`);
  process.stdout.write(`  max_rounds=${args.maxRounds}  max_retries=${args.maxRetries}\n`);
  process.stdout.write(`  serve_port=${args.servePort}  run_timeout=${args.runTimeoutMs / 1000}s\n`);
  process.stdout.write(`${bar}\n\n`);
}

function printSuccess(loop: LoopState): void {
  const bar = "*".repeat(70);
  const total = ((Date.now() - loop.startTime) / 1000).toFixed(1);
  process.stdout.write(`\n${bar}\n`);
  process.stdout.write(`  ✅ WORKFLOW COMPLETED SUCCESSFULLY\n`);
  process.stdout.write(`  Total rounds: ${loop.roundNum}\n`);
  process.stdout.write(`  Total time: ${total}s\n`);
  process.stdout.write(`  Recovery attempts: ${loop.recoveryCount}\n`);
  process.stdout.write(`${bar}\n`);
}

function printEscalation(loop: LoopState, decision: Decision): void {
  const bar = "!".repeat(70);
  const total = ((Date.now() - loop.startTime) / 1000).toFixed(1);
  const d = decision.blockedDetails;
  process.stdout.write(`\n${bar}\n`);
  process.stdout.write(`  🚫 WORKFLOW BLOCKED — HUMAN INTERVENTION REQUIRED\n`);
  process.stdout.write(`  Total rounds: ${loop.roundNum}\n`);
  process.stdout.write(`  Total time: ${total}s\n`);
  process.stdout.write(`  Recovery exhausted: ${loop.recoveryCount}/${DEFAULTS.maxRetries}\n`);
  if (d) {
    process.stdout.write(`\n  Blocked scope: ${d.scope}\n`);
    process.stdout.write(`  Internal attempts: ${d.attempts}/${d.maxAttempts}\n`);
    process.stdout.write(`  Strategies tried: ${d.strategiesTried.length} items\n`);
    d.strategiesTried.forEach((s, i) =>
      process.stdout.write(`    ${i + 1}. ${s}\n`),
    );
  }
  process.stdout.write(`\n  Next steps:\n`);
  process.stdout.write(`    1. Read .opencode/harness/state/${loop.history[0]?.round ? "" : ""}.\n`);
  process.stdout.write(`    2. Review the blocking error reports in evidence/\n`);
  process.stdout.write(`    3. Run: opencode run -c "fix the issue described in ..."\n`);
  process.stdout.write(`    4. Re-run this script: --resume --module <name>\n`);
  process.stdout.write(`${bar}\n`);
}

function printMaxRounds(loop: LoopState): void {
  const bar = "!".repeat(70);
  process.stdout.write(`\n${bar}\n`);
  process.stdout.write(`  ⚠ MAX ROUNDS EXCEEDED\n`);
  process.stdout.write(`  The workflow did not reach a terminal state.\n`);
  process.stdout.write(`  Consider increasing --max-rounds or investigating manually.\n`);
  process.stdout.write(`${bar}\n`);
}

function printUsage(): void {
  process.stdout.write(`
loop-orchestrator.ts — Harness Workflow Autonomous Loop

Usage:
  bun .opencode/harness/scripts/loop-orchestrator.ts --module <name> --plan <path>
  bun .opencode/harness/scripts/loop-orchestrator.ts --resume --module <name>

Options:
  --module NAME       Module name (matches {module}-workflow-state.json)
  --plan PATH         Plan file path (required on first run)
  --resume            Resume from existing state.json
  --max-rounds N      Safety cap on orchestrator rounds (default: ${DEFAULTS.maxRounds})
  --max-retries N     Max blocked→recovery attempts (default: ${DEFAULTS.maxRetries})
  --state-dir PATH    Override state directory (default: ${DEFAULTS.stateDir})
  --serve-port PORT   opencode serve port (default: ${DEFAULTS.servePort})
  --run-timeout SECS  Per-run timeout (default: ${DEFAULTS.runTimeoutMs / 1000})
  --dry-run           Print prompts without executing
  --cli CMD           opencode CLI command (default: ${DEFAULTS.cli})

How it works:
  1. Starts \`opencode serve\` in background
  2. Creates a session, sends initial prompt via prompt_async()
  3. Calls session.wait() — blocks until agent loop finishes
  4. Reads state.json (the contract with harness-dev-workflow skill)
  5. Decides:
       status=completed      → EXIT 0
       status=blocked        → send recovery prompt (new session) → repeat step 3
       status=running        → send continue prompt (same session) → repeat step 3
       no state change ×2    → EXIT 2 (stalled)
  6. If blocked × maxRetries → EXIT 1 (human intervention)

State contract:
  The skill writes state.json at every phase transition. This script reads
  it as the single source of truth. The decision engine never guesses —
  it only acts on what state.json reports.

Exit codes:
  0 = workflow completed successfully
  1 = blocked and recovery exhausted (human intervention required)
  2 = other error (timeout, max rounds, stall)
`);
}

// ============================================================================
// Graceful Shutdown
// ============================================================================

let activeOrchestrator: LoopOrchestrator | null = null;

function setupSignalHandlers(): void {
  const handler = () => {
    process.stdout.write("\n\n  ⚠ Orchestrator interrupted. Server will be cleaned up ...\n");
    process.exit(3);
  };

  process.on("SIGINT", handler);
  process.on("SIGTERM", handler);
}

// ============================================================================
// Entry Point
// ============================================================================

async function main(): Promise<void> {
  setupSignalHandlers();

  const args = parseArgs(process.argv.slice(2));

  // Ensure state dir exists
  await mkdir(args.stateDir, { recursive: true }).catch(() => {});

  // Handle --resume: validate state file exists
  if (args.resume) {
    const stateDir = resolve(args.stateDir);
    const entries = await readdir(stateDir).catch((): string[] => []);
    const stateFilename = `${args.module}-workflow-state.json`;
    const hasState = entries.includes(stateFilename);
    if (!hasState) {
      process.stderr.write(
        `  ❌ --resume: no state file found for module '${args.module}'\n` +
        `     Looked in: ${args.stateDir}\n`,
      );
      process.exit(2);
    }

    // Read current status
    const raw = await readFile(join(stateDir, stateFilename), "utf-8");
    const state = JSON.parse(raw) as StateFile;
    if (state.status === "completed") {
      process.stdout.write("  ✅ State already completed — nothing to do\n");
      process.exit(0);
    }
    process.stdout.write(
      `  Resuming: phase=${state.workflow.phase}  status=${state.status}\n`,
    );
  } else if (!args.plan) {
    process.stderr.write("  ❌ --plan is required on first run (omit with --resume)\n");
    process.exit(2);
  }

  // Run
  const orchestrator = new LoopOrchestrator(args);
  activeOrchestrator = orchestrator;
  const exitCode = await orchestrator.execute();
  activeOrchestrator = null;
  process.exit(exitCode);
}

main().catch((err) => {
  process.stderr.write(`\n  ❌ Orchestrator crashed: ${err}\n`);
  process.exit(2);
});
