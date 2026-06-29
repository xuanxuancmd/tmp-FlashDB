# Acceptance Spec Template

> Back to [SKILL.md](../SKILL.md)

This file provides the detailed template and examples for Phase 4 acceptance spec generation.

---

## Template

```markdown
# Acceptance Spec: {Feature Name}

## User Story

As a {actor},
I want to {capability},
So that {business_value}.

## Scenarios

### Happy Path

#### Scenario: {scenario title}

**Given** {precondition state}
**When** {trigger action}
**Then** {observable outcome}
**And** {observable outcome}

### Sad Path

#### Scenario: {scenario title}

**Given** {precondition state}
**When** {trigger action}
**Then** {observable outcome}

### Edge Cases

#### Scenario: {scenario title}

**Given** {precondition state}
**When** {trigger action}
**Then** {observable outcome}

### State Transitions (if applicable)

#### Scenario: {legal transition title}

**Given** current state is {State A}
**When** {trigger event}
**Then** state transitions to {State B}

#### Scenario: {illegal transition title}

**Given** current state is {State A}
**When** attempting {trigger event}
**Then** operation is rejected, state remains {State A}

## Acceptance Criteria

1. {specific, testable requirement}
2. {specific, testable requirement}
3. {specific, testable requirement}

## Non-Functional Requirements (if applicable)

- **Performance**: {specific metric}
- **Security**: {specific requirement}
- **Reliability**: {specific guarantee}

## Scenario Coverage Matrix

| BL Section | Scenario count | Covered |
|---|---|---|
| Main Flow (Happy Path) | N | ✅ |
| Decision Rules | N | ✅ |
| State Transitions | N | ✅ |
| Exceptions/Edge Cases | N | ✅ |
| Side Effects | N | ✅ |
```

---

## Section Writing Guide

### User Story

- Use business language, for a product manager audience
- `actor`: who triggers/uses this capability (from BL Section 2)
- `capability`: what this Feature lets the actor do (from BL Section 1)
- `business_value`: why this capability is needed (business value)

### Scenarios

- **Happy Path**: normal flow, from BL Section 4 (Main Flow)
- **Sad Path**: error paths, from BL Section 8 (Exceptions)
- **Edge Cases**: boundary conditions, from BL Section 8 (Edge Cases)
- **State Transitions**: state changes, from BL Section 6

**Scenario format notes**:
- This phase uses markdown-format Given/When/Then, **NOT** Gherkin syntax
- `**Given**` / `**When**` / `**Then**` / `**And**` are marked in bold
- Each scenario focuses on one business rule
- Then describes observable outcomes, not internal state

| ❌ Not observable (internal state) | ✅ Observable (external result) |
|---|---|
| 1 row in the database | via API query, resource exists |
| struct field value is 5 | count field in API response is 5 |
| private method was called | system sent a notification message |

### Acceptance Criteria

- Each starts with a verb
- Measurable (can be judged pass/fail)
- Focus on "what" not "how"
- Explicitly include business rules
- Cover error scenarios

### Non-Functional Requirements

- Only fill in when the BL document has relevant information
- Performance: specific metric (e.g., "response time < 100ms")
- Security: specific requirement (e.g., "all operations require authentication")
- Reliability: specific guarantee (e.g., "write operations are atomic")

### Scenario Coverage Matrix

- Ensure each BL Section has corresponding scenarios
- Used for completeness check when generating .feature in Phase 5

---

## Example: File Parser Acceptance Spec

```markdown
# Acceptance Spec: File Parsing and Loading

## User Story

As a caller module,
I want to read a binary-format file from disk and parse it into an in-memory structured document object,
So that subsequent operations can use the structured document data.

## Scenarios

### Happy Path

#### Scenario: Parse valid file returns complete document object

**Given** a valid binary file with correct magic word and version number
**When** calling parse_file to parse the file
**Then** the returned document object contains all data sections
**And** consumed_bytes equals the total file size

#### Scenario: Skip section on CRC validation failure

**Given** a file where one data section's CRC validation fails
**When** calling parse_file to parse the file
**Then** the section with CRC failure is skipped
**And** other sections are parsed normally and included in the document object

### Sad Path

#### Scenario: Reject parsing when magic word does not match

**Given** a file whose header magic word does not match the expected value
**When** calling parse_file to parse the file
**Then** returns ERR_FORMAT error
**And** no document object memory is allocated

#### Scenario: Reject parsing when version number is unsupported

**Given** a file whose version number is not in the supported version list
**When** calling parse_file to parse the file
**Then** returns ERR_VERSION error

### Edge Cases

#### Scenario: Empty file is rejected

**Given** a file whose size is 0 bytes
**When** calling parse_file to parse the file
**Then** returns ERR_EMPTY error

#### Scenario: File size exceeding limit is rejected

**Given** a file whose size exceeds MAX_FILE_SIZE (16MB)
**When** calling parse_file to parse the file
**Then** returns ERR_TOO_LARGE error

#### Scenario: Truncated file is detected

**Given** a file whose actual data is less than the declared section size
**When** calling parse_file to parse the file
**Then** returns ERR_TRUNCATED error

### State Transitions

#### Scenario: Legal transition from IDLE to READING_HEADER

**Given** the parser is in IDLE state
**When** calling parse_file and the file exists with valid size
**Then** state transitions to READING_HEADER

#### Scenario: Cannot return from COMPLETED to IDLE

**Given** the parser is in COMPLETED state
**When** attempting to call parse_file again
**Then** the operation is rejected, state remains COMPLETED

## Acceptance Criteria

1. Valid file parsing returns a document object containing all data sections
2. Returns ERR_FORMAT when magic word does not match, without allocating memory
3. Returns ERR_VERSION when version number is unsupported
4. Returns ERR_EMPTY for empty files
5. Returns ERR_TOO_LARGE when file exceeds 16MB
6. Returns ERR_TRUNCATED for truncated files
7. Skips section on CRC failure without affecting other sections
8. State is COMPLETED after parsing, cannot re-parse
9. State is FAILED after parsing failure, cannot recover
10. consumed_bytes + remaining_bytes always equals the total file size

## Non-Functional Requirements

- **Performance**: 16MB file parsing time < 100ms
- **Reliability**: parsing is a read-only operation, does not modify the original file

## Scenario Coverage Matrix

| BL Section | Scenario count | Covered |
|---|---|---|
| Main Flow (Happy Path) | 2 | ✅ |
| Decision Rules | 4 | ✅ |
| State Transitions | 2 | ✅ |
| Exceptions/Edge Cases | 4 | ✅ |
| Side Effects | 1 | ✅ |
```

---

## Spec to .feature Conversion Mapping

Phase 5 converts this Spec into a .feature file. The mapping:

| Spec element | .feature element |
|---|---|
| User Story | Feature title + description |
| Happy Path scenarios | Scenario (or Background + Scenario) |
| Sad Path scenarios | Scenario |
| Edge Cases scenarios | Scenario or Scenario Outline + Examples |
| State Transitions scenarios | Scenario |
| Acceptance criteria | Distributed across Scenario Then steps |
| Repeated-structure scenario groups | Scenario Outline + Examples |

**Conversion notes**:
- Spec `**Given**` / `**When**` / `**Then**` converts to Gherkin `Given` / `When` / `Then`
- Spec scenario titles convert to the title after `Scenario:`
- If multiple scenarios differ only in data, merge into `Scenario Outline` + `Examples`
- .feature content uses Chinese; Gherkin keywords remain in English
