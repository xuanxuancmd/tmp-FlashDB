---
name: reverse-engineering-test-spec
description: >-
  Reverse-engineers business logic from an existing codebase and generates FT/E2E-level
  acceptance scenarios. Produces a human-readable acceptance spec (markdown) and a Cucumber .feature file.   Extraction granularity is the business capability (Feature), avoiding single-function test explosion.
  Trigger phrases: reverse-engineer test scenarios, generate acceptance cases from code,
  reverse-generate feature files, extract test scenarios from code, code behavior snapshot,
  characterization test, reverse acceptance criteria, FT-level test scenarios,
  E2E-level acceptance cases, reverse specification extraction.
---

# reverse-engineering-test-spec

## Responsibility

**What it does:**
- Reverse-engineers business logic (BL) from an existing codebase at the **Feature** (business-capability) granularity
- Runs a 5-phase pipeline: verified BL docs → acceptance spec → .feature acceptance scenarios
- Phase 4 produces a human-readable acceptance spec (markdown)
- Phase 5 references the `harness-bdd-design` skill's Gherkin rules to produce an executable .feature file
- Final output is submitted to the user for review

**What it does NOT do:**
- ❌ Does not generate test code (Step Definitions, World management — handled by `harness-bdd-coding` or humans)
- ❌ Does not produce single-function-level test scenarios (granularity is the Feature, not the function)
- ❌ Does not run the code under test (pure static analysis)
- ❌ Does not modify source code (read-only on source; writes only BL docs, spec, and .feature files)

**Division of labor with other skills:**

| Role | Responsibility |
|------|----------------|
| **This skill (reverse-engineering-test-spec)** | Reverse code → BL docs → BL validation → acceptance spec → .feature scenarios |
| **harness-bdd-design** | Gherkin rules authority (BRIEF, observable outcomes, step counts, anti-patterns) — referenced by this skill in Phase 5 |
| harness-bdd-coding / human | Write Step Definitions and automation code from .feature files |

---

## Core Principles

### Principle 1: Never confuse code structure with business structure

Technical structure: `view → serializer → service → provider → model`
Business structure: `validate request → determine eligibility → choose route → execute lookup → normalize result → update record`

**What is extracted is the business structure; the technical structure serves only as evidence.**

### Principle 2: Feature granularity, not function granularity

| ✅ Correct granularity (Feature-level) | ❌ Wrong granularity (function-level) |
|---|---|
| "Order creation flow" | "create_order() function" |
| "FlashDB KVDB initialization" | "fdb_kvdb_init() function" |
| "Connector config validation" | "validate_config() function" |
| "File stream writing" | "write_stream() function" |

**FT-level acceptance scenarios verify end-to-end behavior through externally observable entry points, not individual functions.**

### Principle 3: BL is the understanding layer, Spec is the contract layer, .feature is the executable layer

```
Source code → [Phase 1-2: BL extraction] → BL docs → [Phase 3: validate+refine] → trusted BL
             → [Phase 4: acceptance spec] → Spec docs → [Phase 5: .feature generation] → .feature files
```

Never jump directly from code to .feature files — the three-layer progression prevents the "enumerate tests per function" explosion.

### Principle 4: Every BL rule must carry code evidence

Rules without code references are not allowed in BL documents. Reference format: `file:line` or `file:line-range`.

### Principle 5: Gherkin rules follow harness-bdd-design

When generating .feature files in Phase 5, **you MUST load and follow all rules of the `harness-bdd-design` skill**:
- BRIEF (Business language / Real data / Intention revealing / Essential / Focused)
- Step-count hard rules (3-5 steps ideal, ≥10 steps violation)
- Observable-outcome principle (Then only verifies externally observable results)
- Scenario Outline + Examples mechanism
- Anti-pattern correction list

This skill does NOT copy those rules — they are the authority of harness-bdd-design.

### Principle 6: .feature file content uses Chinese

Business terms, scenario descriptions, and step content use Chinese. Gherkin keywords (Feature / Scenario / Given / When / Then / And / Examples) remain in English. Example:

```gherkin
Feature: 文件解析与加载

  Scenario: 解析合法文件返回完整文档对象
    Given 一个合法的二进制文件，包含正确的 magic word 和版本号
    When 调用 parse_file 解析该文件
    Then 返回的文档对象包含所有数据段
    And consumed_bytes 等于文件总大小
```

---

## Applicable Scenarios

- **Code migration / translation projects**: lock down source-side behavior expectations as acceptance criteria for the target side
- **Before major refactoring**: freeze current behavior for regression verification after refactoring
- **Legacy system test backfill**: produce FT/E2E-level acceptance scenarios for historical code
- **Understanding an unfamiliar codebase**: quickly grasp "what does this codebase do" through BL extraction
- **Any situation requiring reverse-engineering acceptance scenarios from code**

---

## Input Parameters

| Parameter | Required | Description |
|-----------|:--------:|-------------|
| `source_dir` | ✅ | Source code directory path |
| `source_lang` | ✅ | Source language identifier (`c` / `java` / `rust` / `python`, etc.) |
| `harness_dir` | ❌ | harness root directory path, default `.opencode/harness/`. Spec output to `{harness_dir}/specs/`, feature output to `{harness_dir}/features/`. **Only `specs/` and `features/` subdirectories are written under this directory.** |
| `output_dir` | ❌ | Output directory for Phase 1-3 intermediate artifacts (feature-inventory.md, bl/, validation-report.md, gaps.md). MUST NOT be under `harness_dir`. Default: `{source_dir}/.reverse-engineering-output/` or a temp directory. |
| `scope` | ❌ | Limit extraction scope: single module / single file / full |
| `feature_hint` | ❌ | User-specified Feature name (e.g., "order creation"); skips Phase 1 auto-discovery |

---

## Output Structure

```
{harness_dir}/                         # default .opencode/harness/
├── specs/                             # Phase 4: acceptance spec (markdown, human-readable)
│   ├── {feature-1}-spec.md
│   └── {feature-2}-spec.md
└── features/                          # Phase 5: .feature scenarios (references harness-bdd-design)
    ├── {feature-1}.feature
    └── {feature-2}.feature

{output_dir}/                          # Phase 1-3 intermediate artifacts (NOT under harness_dir)
├── feature-inventory.md               # Phase 1: Feature inventory + priorities
├── bl/                                # Phase 2-3: business logic documents
│   ├── {feature-1}.md
│   └── {feature-2}.md
├── validation-report.md               # Phase 3: BL validation report
└── gaps.md                            # Phase 3: gap records
```

> **IMPORTANT**: Only `specs/` and `features/` are written under `{harness_dir}/`. All Phase 1-3 intermediate artifacts (feature-inventory.md, bl/, validation-report.md, gaps.md) are written to `{output_dir}/` which MUST NOT be under `{harness_dir}/`.

---

## Extraction Pipeline (5 Phases)

### Phase 1: Feature Discovery

> Start from behavior-change boundaries, not from a uniform top-down scan of the code.

**Purpose**: Identify business capabilities (Features) in the codebase, not enumerate functions.

**Steps**:

1. **Identify the public API surface**: exported functions, interface definitions, public methods. These are the system's interaction points with the external world
2. **Identify entry points**: API endpoints, CLI commands, event handlers, scheduled tasks. These are the starting points of business flows
3. **Identify state boundaries**: where state is created, modified, or persisted. These are key behavior-change points
4. **Identify I/O boundaries**: disk reads/writes, network communication, hardware interaction. These are observation points for system side effects
5. **Cluster into Features**: group related API surface + entry points into a business capability. A Feature should answer "what business capability does this system provide?"

**Feature identification example (generic domain — file parser)**:

```
Feature: File parsing and loading
  ├─ Entry point: parse_file(path) → Document
  ├─ API: validate_header(), read_sections(), build_document()
  └─ I/O: reads file from disk

Feature: Document serialization
  ├─ Entry point: serialize(doc) → bytes
  ├─ API: write_header(), write_sections(), compute_checksum()
  └─ I/O: writes to disk
```

**Output**: `{output_dir}/feature-inventory.md`

```markdown
# Feature Inventory

## Feature 1: {Feature name}
- **Entry point**: {file:line} — {function/endpoint name}
- **API surface**: {function list}
- **State boundaries**: {state-change points}
- **I/O boundaries**: {I/O operations}
- **Priority**: critical | high | medium | low
- **Migration impact**: {if a migration project, note whether in migration scope}
```

**Feature priority ranking**:
- **critical**: core business capability; system is unusable without it
- **high**: important business capability; affects user experience
- **medium**: auxiliary function
- **low**: edge-case scenario

---

### Phase 2: BL Extraction

> Extract a business-logic document, translating technical language into business language.

**Purpose**: For each Feature, extract a structured BL document.

**Steps**:

1. **Trace the execution path**: from the entry point, follow the call chain: `entry → validation → business logic → side effect → response`
2. **Extract business decisions**: identify and record validation rules, routing logic, state changes, error handling, retry/fallback
3. **Separate technical from business**: rewrite technical mechanisms in business language

| Technical language (❌) | Business language (✅) |
|---|---|
| `calls function X` | `attempts provider A, then provider B if A fails` |
| `updates status field` | `marks task as completed` |
| `throws ValidationError` | `rejects invalid input format` |
| `check quota >= cost` | `verifies sufficient credits before operation` |

4. **Produce the BL document using the 11-section template**

**BL 11-section template** (detailed template in [references/bl-template.md](references/bl-template.md)):

| # | Section | Content |
|---|---------|---------|
| 1 | Business Purpose | What business capability this Feature provides |
| 2 | Actors | Who triggers/uses this capability |
| 3 | Preconditions | What must be true before it runs |
| 4 | Main Flow | Business flow (NOT code structure!) |
| 5 | Decision Rules | Explicit if/then rules |
| 6 | State Transitions | How state changes |
| 7 | Side Effects | External side effects (I/O, persistence, notifications) |
| 8 | Exceptions/Edge Cases | Exceptions and boundaries |
| 9 | Data Persistence | Data written/read |
| 10 | Ambiguities | What cannot be inferred from code |
| 11 | Code References | file:line evidence index |

**Output**: `{output_dir}/bl/{feature-name}.md`

**Quality self-check**:
- ✅ All sections have content (when N/A, explicitly state "N/A — reason")
- ✅ Main Flow uses business language, not code call chains
- ✅ Decision Rules are explicit if/then, not narrative
- ✅ Every rule carries file:line evidence
- ✅ State Transitions cover both legal and illegal transitions
- ✅ Ambiguities honestly record what is "unknown"

---

### Phase 3: BL Validation & Refinement

> Validate BL against code, find gaps, refine vague rules into deterministic ones.

**Purpose**: The BL from Phase 2 is "candidate"; this phase validates alignment with code and refines vague rules.

#### 3a. BL Validation

For each rule in the BL document, search for code evidence and judge alignment status:

| Status | Meaning |
|--------|---------|
| **Implemented** | Code directly implements the BL rule |
| **Partially Implemented** | Some scenarios covered, edges missing |
| **Contradicted** | Code contradicts the BL (BL needs correction) |
| **Not Found** | Cannot be found in code (BL may be speculation) |
| **Hidden Behavior** | Code does something the BL doesn't record (BL needs supplement) |

**Validation matrix**:

```markdown
| BL Rule | Status | Evidence (file:line) | Notes |
|---------|--------|---------------------|-------|
| {rule description} | Implemented | {file:line} | {notes} |
| {rule description} | Contradicted | {file:line} | {contradiction details} |
```

#### 3b. Gap Analysis

Systematically check 10 categories of BL gaps (detailed checklist in [references/validation-guide.md](references/validation-guide.md)):

1. Entry/exit conditions complete?
2. State definitions exhaustive?
3. State transitions covered (including error transitions)?
4. Exception paths handled?
5. Side effects explicit?
6. Retry/fallback logic deterministic?
7. Concurrent operations specified?
8. Terminology consistent?
9. Actor responsibilities clear?
10. Time/timeout constraints specified?

#### 3c. BL Refinement

Rewrite vague rules into deterministic rules:

| Vague (❌) | Deterministic (✅) |
|---|---|
| "the system retries" | "retries up to 3 times with exponential backoff (1s, 2s, 4s)" |
| "if there's an error" | "if the provider returns 500/502/503/504" |
| "successful response" | "HTTP 200-299" |
| "after processing" | "after the provider returns success or failure, excluding timeout" |
| "may charge" | "charges synchronously before provider submission" |

**Refinement focus**:
- Eliminate vague words (may, might, typically, usually, if needed)
- All timeouts/retries have explicit values
- All conditions are explicit (no "as needed")
- Separate Policy (what) from Mechanism (how)
- Extract complex conditional logic into Decision Tables

**Output**:
- `{output_dir}/validation-report.md` — validation matrix + coverage statistics
- `{output_dir}/gaps.md` — gap list + severity grading + fix suggestions
- `{output_dir}/bl/{feature-name}.md` updated to the refined version

---

### Phase 4: Acceptance Spec Generation

> Derive an acceptance spec from the trusted BL.

**Purpose**: Convert trusted BL into a human-readable acceptance spec document (markdown), containing User Story, Given/When/Then scenarios, acceptance criteria, and NFRs.

#### Prerequisites

- Phase 3 BL validation passed (no Contradicted rules, or corrected)
- BL is refined (no vague words)

#### Steps

1. **Derive User Story from BL**

   For each Feature's BL document, express it in this format:

   ```markdown
   As a {actor},
   I want to {capability},
   So that {business_value}.
   ```

2. **Derive Given/When/Then scenarios from BL (markdown format, NOT Gherkin)**

   | BL Section | Derive as |
   |---|---|
   | Main Flow | Happy path scenarios |
   | Decision Rules | Parameterized scenario groups (one per rule) |
   | State Transitions | State-transition scenarios (legal + illegal) |
   | Exceptions/Edge Cases | Exception paths / boundary scenarios |
   | Preconditions | Given preconditions |
   | Side Effects | Then observable assertions |

   **Scenario format (markdown)**:

   ```markdown
   ### Scenario: {scenario title}

   **Given** {precondition state}
   **When** {trigger action}
   **Then** {observable outcome}
   **And** {observable outcome}
   ```

   > Note: This phase outputs markdown-format scenario descriptions, NOT Gherkin syntax for .feature files. Phase 5 converts them to .feature.

3. **Generate acceptance criteria**

   For each User Story, list testable acceptance criteria:

   ```markdown
   ## Acceptance Criteria
   1. {specific, testable requirement}
   2. {specific, testable requirement}
   ```

4. **Supplement non-functional requirements (if applicable)**

   ```markdown
   ## Non-Functional Requirements
   - **Performance**: {specific metric}
   - **Security**: {specific requirement}
   - **Reliability**: {specific guarantee}
   ```

5. **Quality self-check**

   - ✅ Each Feature produces one Spec document
   - ✅ Spec contains User Story + scenarios + acceptance criteria
   - ✅ Scenarios use business language, not technical implementation details
   - ✅ Acceptance criteria are testable (can be judged pass/fail)
   - ✅ Scenarios cover happy path + sad path + edge case

#### Output

`{harness_dir}/specs/{feature-name}-spec.md`

**Spec document structure** (detailed template in [references/acceptance-spec-template.md](references/acceptance-spec-template.md)):

```markdown
# Acceptance Spec: {Feature name}

## User Story
As a {actor}, I want to {capability}, So that {business_value}.

## Scenarios

### Happy Path
Scenario: {title}
Given / When / Then ...

### Sad Path
Scenario: {title}
Given / When / Then ...

### Edge Cases
Scenario: {title}
Given / When / Then ...

## Acceptance Criteria
1. ...
2. ...

## Non-Functional Requirements (if applicable)
- Performance: ...
- Security: ...
```

---

### Phase 5: .feature File Generation

> References the `harness-bdd-design` skill's Gherkin rules to generate an executable .feature file from the acceptance spec.

**Purpose**: Convert the Phase 4 Spec document into a Cucumber/Gherkin-format .feature file for automated test execution.

#### Prerequisites

- Phase 4 Spec document completed
- Spec document scenarios cover happy path + sad path + edge case

#### Fallback: harness-bdd-design not found

**Before executing Phase 5, you MUST check whether the `harness-bdd-design` skill is available** (via the `skill` tool).

- **If harness-bdd-design is available**: proceed with Phase 5, generate .feature files
- **If harness-bdd-design is NOT available**: **skip Phase 5, submit only the Phase 4 Spec document to the user**

Fallback submission message:

```
## Reverse acceptance scenario generation complete (fallback mode)

### Output inventory
| Feature | Spec | .feature | Note |
|---------|------|----------|------|
| {name} | specs/{name}-spec.md | — | harness-bdd-design not found, skipping .feature generation |

### Statistics
- Feature count: N
- Spec scenario count: N
- .feature scenario count: 0 (fallback mode)

### Note
The harness-bdd-design skill was not found; cannot generate Gherkin-format .feature files.
The generated Spec document (markdown Given/When/Then scenarios) can be used for:
1. Manually writing .feature files
2. Re-running Phase 5 after installing the harness-bdd-design skill
3. Directly as an acceptance spec document for business review

Please review the Spec output. To generate .feature files, ensure the harness-bdd-design skill is installed.
```

#### Steps (executed when harness-bdd-design is available)

1. **Load the harness-bdd-design skill**

   **You MUST load the `harness-bdd-design` skill** and follow all its rules:
   - §1 BRIEF (scenarios ≤ 5 steps ideal, ≥ 10 steps violation)
   - §2 Given/When/Then discipline (strictly one-directional, 1 When-Then pair)
   - §3 Observable-outcome principle (Then only verifies externally observable results)
   - §4 Scenario Outline + Examples (deterministic parameterization)
   - §5 Anti-pattern correction (10 anti-patterns)
   - §6 FT-level templates

2. **Convert Spec to .feature**

   Convert the Phase 4 markdown scenarios into Gherkin syntax. **.feature file content uses Chinese** (business terms, scenario descriptions, and step content in Chinese; Gherkin keywords remain in English):

   ```gherkin
   Feature: {Feature name in Chinese}

     Background:
       Given {system-level precondition in Chinese}

     Scenario: {Happy path — business rule title in Chinese}
       Given {precondition state in Chinese}
       When {trigger action in Chinese}
       Then {observable outcome in Chinese}
       And {observable outcome in Chinese}

     Scenario Outline: {parameterized scenario — business rule title in Chinese}
       Given {precondition state with <placeholder>}
       When {trigger action with <placeholder>}
       Then {observable outcome with <placeholder>}

       Examples:
         | column1 | column2 |
         | value1  | value2  |
   ```

3. **Self-check (follow harness-bdd-design §7 review checklist)**

   After generating each .feature file, execute the harness-bdd-design §7 review checklist:

   | Dimension | Check |
   |-----------|-------|
   | FT-level granularity | Interacts via external entry points; does not directly call internal functions |
   | BRIEF | Most scenarios ≤ 5 steps; all ≤ 9 steps |
   | Single behavior | Each scenario has exactly 1 When-Then pair |
   | Observable | Each Then is an observable outcome (not DB/internal state) |
   | Deterministic | Steps + Examples guarantee consistent results across runs |
   | Business rules | Title reflects business rule; no missing exceptions/boundaries |

   **Mandatory rejection criteria** (violation of any one means no output):
   - Then verifies internal state (DB rows, struct fields, private methods)
   - Function-level assertions (`method() ==`)
   - Scenario exceeds 9 steps without splitting
   - Vague outcome words ("正确", "成功", "正常")

#### Output

`{harness_dir}/features/{feature-name}.feature`

---

### Final Delivery: Submit to User for Review

After Phase 5 completes (or after Phase 4 in fallback mode), submit the output to the user for review.

#### Normal mode (harness-bdd-design available)

1. **`{harness_dir}/specs/{feature-name}-spec.md`** — Acceptance spec (human-readable, for business/PM review)
2. **`{harness_dir}/features/{feature-name}.feature`** — .feature acceptance scenarios (executable, for dev/QA)

**Submission format**:

```
## Reverse acceptance scenario generation complete

### Output inventory
| Feature | Spec | .feature | Scenario count |
|---------|------|----------|----------------|
| {name} | specs/{name}-spec.md | features/{name}.feature | N |

### Statistics
- Feature count: N
- BL rule count: N
- Validation coverage: XX%
- Spec scenario count: N
- .feature scenario count: N

Please review the above output. Once confirmed, the .feature files can be handed off to the automation phase.
```

---

## References

| File | Purpose |
|------|---------|
| [references/bl-template.md](references/bl-template.md) | Detailed BL 11-section template + examples |
| [references/validation-guide.md](references/validation-guide.md) | BL validation matrix + 10-category gap checklist + refinement patterns |
| [references/acceptance-spec-template.md](references/acceptance-spec-template.md) | Acceptance spec document template + examples |

> **Gherkin rules are NOT in this skill's references** — they are the authoritative content of the `harness-bdd-design` skill. Phase 5 MUST load the harness-bdd-design skill to obtain them.

---

## Prohibitions

1. ❌ Modifying the code under test (this skill is read-only on source)
2. ❌ Using "正确", "合适", "良好", "成功" or other unverifiable words in BL docs, spec, or .feature files
3. ❌ Generating test code / Step Definitions / automation code
4. ❌ Running the code under test
5. ❌ Writing BL rules without code evidence
6. ❌ Writing "understood intent" into BL (extract only what the code actually does)
7. ❌ Skipping any Phase (you may mark "N/A" but must explicitly state why)
8. ❌ Generating .feature files in Phase 5 without loading the harness-bdd-design skill (if the skill is unavailable, fall back to spec-only output)
9. ❌ Generating single-function-level test scenarios (granularity is the Feature, not the function)
10. ❌ Verifying internal state in .feature Then steps (DB rows, struct fields, private methods)
11. ❌ Using English for .feature file content (business terms and scenario descriptions MUST use Chinese; only Gherkin keywords remain in English)
12. ❌ Skipping the "submit to user for review" step

## Mandates

1. ✅ Every BL rule must carry code evidence (file:line)
2. ✅ BL Main Flow uses business language, not code call chains
3. ✅ BL Decision Rules are explicit if/then, not narrative
4. ✅ Phase 3 validation matrix covers every rule in the BL document
5. ✅ Every gap in Phase 3 gaps.md carries a "why it cannot be determined" reason
6. ✅ Phase 5 MUST check whether the harness-bdd-design skill is available; if unavailable, fall back to spec-only output
7. ✅ When harness-bdd-design is available, execute its §7 review checklist after generating each .feature file
8. ✅ Scenario Then steps only verify externally observable outcomes
9. ✅ Each Scenario has exactly 1 When-Then pair
10. ✅ Scenarios exceeding 9 steps MUST be split
11. ✅ .feature file content uses Chinese (Gherkin keywords remain in English)
12. ✅ Submit the spec + .feature output inventory to the user for review after Phase 5

---

## Quick Reference: Execution Steps

```
1. Phase 1: Feature Discovery → {output_dir}/feature-inventory.md (Feature inventory + priorities)
2. Phase 2: BL Extraction → {output_dir}/bl/{feature}.md (11-section business logic document)
3. Phase 3: BL Validation + Refinement → {output_dir}/validation-report.md + {output_dir}/gaps.md + updated {output_dir}/bl/
4. Phase 4: Acceptance Spec Generation → {harness_dir}/specs/{feature}-spec.md
5. Phase 5: .feature File Generation (load harness-bdd-design) → {harness_dir}/features/{feature}.feature
6. Submit to user for review: output inventory + statistics summary
```
