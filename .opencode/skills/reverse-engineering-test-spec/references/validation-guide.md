# BL Validation & Refinement Guide

> Back to [SKILL.md](../SKILL.md)

This file provides the validation matrix, gap checklist, and refinement patterns for Phase 3.

---

## 1. BL Validation Matrix

For each rule in the BL document, search for code evidence and judge alignment status.

### Status Definitions

| Status | Meaning | Action |
|--------|---------|--------|
| **Implemented** | Code directly implements the BL rule | Pass |
| **Partially Implemented** | Some scenarios covered, edges missing | Note missing scenarios; supplement BL |
| **Contradicted** | Code contradicts the BL | Correct BL to match actual code behavior |
| **Not Found** | Cannot be found in code | Delete the rule or mark as speculation |
| **Hidden Behavior** | Code does something the BL doesn't record | Supplement BL |

### Matrix Format

```markdown
# Validation Report: {Feature Name}

## Coverage Statistics
- Total rules: N
- Implemented: X (XX%)
- Partially Implemented: Y (YY%)
- Contradicted: Z (ZZ%)
- Not Found: W (WW%)
- Hidden Behavior: V (VV%)

## Validation Matrix

| BL Rule | Section | Status | Evidence (file:line) | Notes |
|---------|---------|--------|---------------------|-------|
| File path must be valid | Preconditions | Implemented | parser.c:42 | Path validation matches BL |
| Skip section on CRC failure | Decision Rules | Implemented | parser.c:90 | Skip logic matches BL |
| Thread safety | Ambiguities | Not Found | — | No concurrency protection in code |
| Truncated file detection | Exceptions | Partially | parser.c:85 | Detected but does not return ERR_TRUNCATED |

## Critical Issues
1. {Contradicted rule}: {contradiction details}
2. {Hidden Behavior}: {code does something BL didn't record}
```

### Validation Tips

- **Search multiple keywords**: the same rule may be implemented with different terms ("timeout", "deadline", "expire")
- **Read actual code**: don't rely on function names; read the implementation body
- **Check side effects**: some rules have implicit side-effect implementations
- **Verify model constraints**: some rules are enforced at the database / config layer
- **Check for multiple implementations**: the same rule may be implemented in multiple places

---

## 2. Gap Checklist (10 Categories)

Systematically check 10 categories of BL gaps.

### A. Entry/Exit Conditions

**Questions**:
- Are all preconditions specified?
- Are all possible exit states defined?
- What happens when preconditions are not met?

**Check for**:
- Missing input validation rules
- Undefined behavior for edge cases (null, empty, invalid values)
- Missing cleanup/rollback on failure

### B. State Definitions

**Questions**:
- Are all possible states explicitly listed?
- Is each state's meaning clear?
- Are states mutually exclusive and collectively exhaustive?

**Check for**:
- References to states not in the state list
- Ambiguous state names ("processed", "handled")
- Missing terminal states
- Undefined initial state

### C. State Transitions

**Questions**:
- For each state, what transitions are allowed?
- Which transitions are explicitly forbidden?
- What triggers each transition?

**Check for**:
- States with no defined transitions
- Transitions mentioned in workflows but not in the state machine
- Missing error-state transitions
- Undefined transition conditions

### D. Exception Paths

**Questions**:
- What happens when external services fail?
- What happens on timeout?
- What happens on invalid input?
- What happens on concurrent operations?

**Check for**:
- Only happy path documented
- Missing error-handling specifications
- Undefined retry behavior
- Undefined rollback mechanisms

### E. Side Effects

**Questions**:
- Is the timing of side effects explicit?
- Are side effects reversible?
- What happens when a side effect fails?

**Check for**:
- Vague timing ("may write", "typically persists")
- Irreversible operations not marked
- Side-effect failure handling undefined

### F. Retry/Fallback Logic

**Questions**:
- What conditions trigger a retry?
- How many retries are allowed?
- What changes between retries?
- When is a fallback used?

**Check for**:
- Vague retry logic ("if needed", "may retry")
- Missing backoff strategies
- Undefined fallback conditions
- Missing circuit-breaker specifications

### G. Concurrent Operations

**Questions**:
- What happens if the same request is submitted twice?
- What happens if conflicting operations occur simultaneously?
- Are there idempotency guarantees?

**Check for**:
- Missing idempotency guarantees
- Undefined behavior for concurrent state changes
- Missing lock/transaction specifications
- Undefined duplicate-request handling

### H. Terminology Consistency

**Questions**:
- Are terms used consistently throughout?
- Do multiple terms refer to the same concept?
- Does the same term mean different things in different contexts?

**Check for**:
- "query", "request", "lookup" used interchangeably
- "success" meaning different things (HTTP 200 vs business success)
- Inconsistent status names

### I. Actor Responsibilities

**Questions**:
- Who/what initiates each action?
- Which system is responsible for each decision?
- Are external actor responsibilities clear?

**Check for**:
- Passive voice without a clear actor ("is processed", "is handled")
- Unclear decision ownership
- Missing external-system responsibilities

### J. Time/Timeout Constraints

**Questions**:
- Are timeouts specified?
- Are there SLA requirements?
- Are there timing-dependent behaviors?

**Check for**:
- Missing timeout values
- Undefined delays/waiting periods
- Undefined expiration rules

### Gap Severity Grading

| Level | Meaning | Action |
|-------|---------|--------|
| **Critical** | Will cause bugs or data loss | Must fix before Phase 4 |
| **Important** | May cause issues | Should fix before Phase 4 |
| **Nice to have** | Improves clarity | Can defer |

---

## 3. BL Refinement Patterns

Rewrite vague rules into deterministic rules.

### Pattern 1: Conditional vagueness → Explicit conditions

**Before**:
```
The system tries another provider when needed.
```

**After**:
```
If the primary provider returns TIMEOUT or CONNECTION_ERROR:
  - The system retries once with the secondary provider
  - The secondary provider is selected based on priority routing rules

If the primary provider returns INVALID_TARGET:
  - No fallback is attempted
  - The request immediately transitions to FAILED
```

### Pattern 2: Timing vagueness → Explicit timeouts

**Before**:
```
The system polls for status until complete.
```

**After**:
```
Status polling behavior:
  - Initial poll: immediately after submission
  - Subsequent polls: every 30 seconds
  - Maximum poll duration: 5 minutes (10 polls)
  - After 10 incomplete polls: transition to STALLED
  - STALLED can be resumed with the CONTINUE operation
```

### Pattern 3: Billing/side-effect vagueness → Deterministic rules

**Before**:
```
Credits are deducted for operations.
```

**After**:
```
Credit deduction rules:
  - Credits are deducted synchronously BEFORE provider submission
  - Deduction amount: Provider.query_cost value
  - If deduction fails (insufficient credits): request is rejected, no provider call made
  - Refunds: No automatic refunds for failed requests
  - Partial success: Full credit amount deducted even if only partial data returned
```

### Pattern 4: State vagueness → Explicit state machine

**Before**:
```
Requests can be pending, processing, or done.
```

**After**:
```
Request state machine:

States:
  - PENDING: Initial state, awaiting validation
  - VALIDATED: Passed validation, awaiting credit check
  - SUBMITTED: Credits deducted, provider called
  - COMPLETED: Provider returned successful result
  - FAILED: Request failed (validation, credit, or provider error)
  - STALLED: Provider timeout, can be retried
  - CANCELLED: Request cancelled by user

Allowed transitions:
  - PENDING → VALIDATED: Input validation passes
  - VALIDATED → SUBMITTED: Sufficient credits, deduction successful
  - SUBMITTED → COMPLETED: Provider returns successful result
  - SUBMITTED → STALLED: Provider timeout (no response within 60s)
  - STALLED → SUBMITTED: User triggers CONTINUE operation
  - (any active state) → CANCELLED: User requests cancellation
  - (any state) → FAILED: Validation fails, insufficient credits, or provider error

Forbidden transitions:
  - COMPLETED → any other state (terminal)
  - FAILED → any state other than CANCELLED
```

### Pattern 5: Complex conditions → Decision Table

**Before**:
```
Different providers are selected based on request type and location.
```

**After**:
```
## Provider Selection Decision Table

| Request Type | Target Region | Has MCC | Primary Provider | Fallback Provider |
|--------------|---------------|---------|------------------|-------------------|
| GEO          | US            | Yes     | ProvA            | ProvB             |
| GEO          | US            | No      | ProvC            | ProvA             |
| GEO          | Non-US        | Yes     | ProvA            | ProvD             |
| GEO          | Non-US        | No      | ProvC            | ProvD             |
| CDR          | Any           | N/A     | ProvE            | ProvF             |

Notes:
- No fallback when the primary provider returns INVALID_TARGET
- Fallback is used when the primary provider returns TIMEOUT or CONNECTION_ERROR
```

### Vague-Word Blacklist

When the following words appear in BL, they **MUST be refined into deterministic statements**:

| Vague word | Refinement direction |
|------------|---------------------|
| may, might | Remove or make deterministic |
| typically, usually | Remove or specify conditions |
| if needed, when appropriate | Specify exact conditions |
| processed, handled | Specify concrete behavior |
| valid, correct | Specify validation rules |
| as needed | Specify exact conditions |
| appropriate error | Specify concrete error codes |
| success | Specify success criteria (HTTP 200-299, etc.) |

---

## 4. Post-Refinement Quality Check

The refined BL should pass the following checks:

- [ ] No vague words (may, might, typically, usually)
- [ ] All timeouts have explicit values
- [ ] All conditions are explicit (no "if needed")
- [ ] All states are defined
- [ ] All transitions are explicitly listed
- [ ] Side-effect rules are deterministic
- [ ] Terminology is consistent throughout
- [ ] Policy is separated from Mechanism
- [ ] Complex logic has Decision Tables
- [ ] Each rule can be directly turned into a test scenario
