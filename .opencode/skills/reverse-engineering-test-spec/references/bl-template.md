# BL 11-Section Template

> Back to [SKILL.md](../SKILL.md)

This file provides the detailed template and examples for Phase 2 BL extraction.

---

## Template

```markdown
# Business Logic: {Feature Name}

## 1. Business Purpose
{What business capability this Feature provides — 1-2 sentences, for non-technical readers}

## 2. Actors
- {Actor 1}: {role description}
- {Actor 2}: {role description}

## 3. Preconditions
1. {precondition 1} — *evidence: `{file}:{line}`*
2. {precondition 2} — *evidence: `{file}:{line}`*

## 4. Main Flow
{Business flow steps, in business language, NOT a code call chain}

1. {business step 1}
2. {business step 2}
3. {business step 3}
...

## 5. Decision Rules
- **If** {condition} **then** {outcome} — *evidence: `{file}:{line}`*
- **If** {condition} **then** {outcome} — *evidence: `{file}:{line}`*

## 6. State Transitions
States:
- {State A}: {description}
- {State B}: {description}

Transitions:
| From | Event | To | Guard | Evidence |
|------|-------|-----|-------|---------|
| {A} | {event} | {B} | {precondition} | `{file}:{line}` |

Forbidden transitions:
- {A} → {C}: {reason}

## 7. Side Effects
- {side effect 1}: {description} — *evidence: `{file}:{line}`*
- {side effect 2}: {description} — *evidence: `{file}:{line}`*

## 8. Exceptions/Edge Cases
- {exception 1}: {handling} — *evidence: `{file}:{line}`*
- {edge case 1}: {handling} — *evidence: `{file}:{line}`*

## 9. Data Persistence
- **Write**: {what data is written where} — *evidence: `{file}:{line}`*
- **Read**: {what data is read from where} — *evidence: `{file}:{line}`*

## 10. Ambiguities
- {item 1 that cannot be inferred from code}: {why it cannot be inferred}
- {item 2 needing human confirmation}: {what needs confirmation}

## 11. Code References
| Reference ID | File | Lines | What it proves |
|---|---|---|---|
| CR-1 | {file} | {line-range} | {what this code segment proves} |
| CR-2 | {file} | {line-range} | {what this code segment proves} |
```

---

## Section Writing Guide

### 1. Business Purpose
- 1-2 sentences, understandable by a product manager
- Answers "what does this Feature let users/the system do"
- No technical implementation details

### 2. Actors
- List all roles that trigger or participate in this Feature
- Include human users, external systems, timers/schedulers
- One sentence per role

### 3. Preconditions
- Conditions that must be true before this Feature runs
- Include: resources initialized, permissions verified, state ready
- Each carries file:line evidence

### 4. Main Flow
- **The most critical Section** — describe the flow in business language, not a code call chain
- Steps are connected by business causality, not function-call relationships
- Each step is a "business action", not a "technical operation"

| ❌ Technical language | ✅ Business language |
|---|---|
| calls validate_input() | validates input format |
| sets status = RUNNING | marks task as running |
| calls provider API | sends request to service provider |
| writes to database | persists result record |

### 5. Decision Rules
- Each rule is an explicit if/then
- Cover: validation rules, routing logic, eligibility checks, termination conditions
- No narrative ("the system checks..."); use rules ("If X then Y")

### 6. State Transitions
- List all states and their meanings
- List all legal transitions (with triggering event and guard)
- List illegal transitions (transition pairs not observed in code)
- If the Feature has no state machine, mark "N/A — this Feature has no state management"

### 7. Side Effects
- Externally observable side effects: I/O operations, network communication, hardware interaction
- Irreversible operations: delete, overwrite, send
- Impact on other systems

### 8. Exceptions/Edge Cases
- Exception paths: invalid input, resource exhaustion, external service failure
- Boundary conditions: empty input, maximum values, zero values, concurrency
- Describe handling for each case

### 9. Data Persistence
- Write: what data goes where (disk file, database, shared memory)
- Read: what data is read from where
- Data format: binary layout, serialization format

### 10. Ambiguities
- **Honestly record what is "unknown"**
- Include: runtime behavior that cannot be inferred from static code, domain-knowledge gaps, speculative content
- Each item explains "why it cannot be inferred"

### 11. Code References
- Aggregate all file:line references
- Each Reference has a unique ID (CR-1, CR-2...)
- State what BL rule this code segment proves

---

## Example: File Parser BL Document

```markdown
# Business Logic: File Parsing and Loading

## 1. Business Purpose
Reads a binary-format file from disk, validates integrity, and parses it into an in-memory structured document object for subsequent operations.

## 2. Actors
- Caller: the module that initiates the parse request via API
- File system: the storage backend providing file data

## 3. Preconditions
1. File path is valid and file exists — *evidence: `parser.c:42`*
2. File size > 0 — *evidence: `parser.c:44`*
3. File size <= MAX_FILE_SIZE (16MB) — *evidence: `parser.c:45`*

## 4. Main Flow
1. Read the file header; validate the magic word and version number
2. Read the section table; determine which data sections the file contains
3. Read each section; validate each section's CRC
4. Assemble the validated data into a document object
5. Return the document object to the caller

## 5. Decision Rules
- **If** magic word does not match **then** reject parsing, return ERR_FORMAT — *evidence: `parser.c:60`*
- **If** version number is unsupported **then** reject parsing, return ERR_VERSION — *evidence: `parser.c:65`*
- **If** section CRC validation fails **then** skip that section, log a warning — *evidence: `parser.c:90`*
- **If** file size exceeds MAX_FILE_SIZE **then** reject parsing, return ERR_TOO_LARGE — *evidence: `parser.c:45`*

## 6. State Transitions
States:
- IDLE: parsing not started
- READING_HEADER: reading file header
- READING_SECTIONS: reading data sections
- COMPLETED: parsing complete, document object available
- FAILED: parsing failed

Transitions:
| From | Event | To | Guard | Evidence |
|------|-------|-----|-------|---------|
| IDLE | parse_file() | READING_HEADER | file exists and size is valid | `parser.c:50` |
| READING_HEADER | header validation passes | READING_SECTIONS | magic word matches | `parser.c:70` |
| READING_HEADER | header validation fails | FAILED | magic word does not match | `parser.c:60` |
| READING_SECTIONS | all sections read | COMPLETED | all section CRCs pass or are skipped | `parser.c:110` |
| COMPLETED | — | — | terminal state | — |
| FAILED | — | — | terminal state | — |

Forbidden transitions:
- COMPLETED → IDLE: document object already returned; cannot re-parse
- FAILED → READING_SECTIONS: cannot recover after failure

## 7. Side Effects
- Disk read: reads file contents from the file system — *evidence: `parser.c:52`*
- Memory allocation: allocates memory for the document object — *evidence: `parser.c:100`*

## 8. Exceptions/Edge Cases
- File does not exist: return ERR_NOT_FOUND — *evidence: `parser.c:42`*
- Empty file (size=0): return ERR_EMPTY — *evidence: `parser.c:44`*
- Truncated file (actual data < declared size): return ERR_TRUNCATED — *evidence: `parser.c:85`*
- Section CRC failure: skip that section, continue parsing other sections — *evidence: `parser.c:90`*

## 9. Data Persistence
- **Read**: reads binary file from disk — *evidence: `parser.c:52`*
- **Write**: none (parsing is a read-only operation)

## 10. Ambiguities
- Whether skipped sections (CRC failure) affect document object completeness: not clear in code; depends on how the caller uses the document object
- Concurrent parsing of the same file: no concurrency protection in code; behavior is undefined

## 11. Code References
| Reference ID | File | Lines | What it proves |
|---|---|---|---|
| CR-1 | parser.c | 42-45 | File existence and size validation |
| CR-2 | parser.c | 50-70 | Header reading and validation flow |
| CR-3 | parser.c | 85-90 | Section reading and CRC validation |
| CR-4 | parser.c | 100-110 | Document object assembly and return |
```
