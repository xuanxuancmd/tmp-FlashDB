# Specification Templates Reference

> 返回 [SKILL.md](../SKILL.md)

本文件提供所有提取产出的标准模板,供提取时参考。

---

## 1. Contract 模板

```markdown
### CONTRACT-{ID}: {inflection point name}

**Source**: `{file}:{line-range}`
**Kind**: API-boundary | State-boundary | IO-boundary
**Confidence**: high | medium | low

**Preconditions**:
1. {caller obligation} — *evidence: `{file}:{line}`*
2. {caller obligation} — *evidence: `{file}:{line}`*

**Postconditions**:
1. When {condition}: {system guarantee} — *evidence: `{file}:{line}`*
2. When {condition}: {system guarantee} — *evidence: `{file}:{line}`*

**Invariants**:
1. {always-true statement} — *evidence: `{file}:{line}`*
```

### 范例: API-boundary Contract

```markdown
### CONTRACT-03: parse_message

**Source**: `parser.c:40-95`
**Kind**: API-boundary
**Confidence**: high

**Preconditions**:
1. `data` 指针非空 — *evidence: `parser.c:42` (`if (data == NULL) return ERR_NULL`)*
2. `len` > 0 — *evidence: `parser.c:43` (`if (len == 0) return ERR_EMPTY`)*
3. `len` <= MAX_MSG_SIZE (65536) — *evidence: `parser.c:44`*

**Postconditions**:
1. When 解析成功: 返回 `Message` 结构,其中 `header.type` ∈ {DATA, CTRL, ACK}, `payload.len` <= `len - HEADER_SIZE` — *evidence: `parser.c:78-82`*
2. When 解析成功: `consumed_bytes == HEADER_SIZE + payload.len` — *evidence: `parser.c:85`*
3. When 校验失败: 返回 `ERR_CHECKSUM`,不修改任何输出字段 — *evidence: `parser.c:60-62`*

**Invariants**:
1. `consumed_bytes + remaining_bytes == len` 在每次调用后成立 — *evidence: `parser.c:90`*
```

### 范例: State-boundary Contract

```markdown
### CONTRACT-07: session_send

**Source**: `session.c:120-180`
**Kind**: State-boundary
**Confidence**: high

**Preconditions**:
1. session 当前状态 ∈ {ACTIVE, RECONNECTING} — *evidence: `session.c:122` (guard clause)*

**Postconditions**:
1. When 发送成功: `sent_count` 增加 1 — *evidence: `session.c:165`*
2. When 网络不可达: session 转为 RECONNECTING 状态,`retry_count` 重置为 0 — *evidence: `session.c:170-172`*
3. When session 状态 ∉ {ACTIVE, RECONNECTING}: 返回 `ERR_INVALID_STATE`,无任何副作用 — *evidence: `session.c:122-123`*

**Invariants**:
1. `sent_count + failed_count == total_attempts` — *evidence: `session.c:165,173`*
```

### 范例: IO-boundary Contract

```markdown
### CONTRACT-12: flush_to_disk

**Source**: `storage.c:200-250`
**Kind**: IO-boundary
**Confidence**: medium

**Preconditions**:
1. `buffer.len` > 0 (有数据需要刷写) — *evidence: `storage.c:202`*
2. fd 是有效的打开文件描述符 — *evidence: invariant maintained by `open_storage()`*

**Postconditions**:
1. When 写入成功: `buffer.len` 重置为 0,文件偏移量前进 `old buffer.len` — *evidence: `storage.c:240-242`*
2. When 磁盘满: 返回 `ERR_DISK_FULL`,`buffer` 内容不变 — *evidence: `storage.c:230`*

**Invariants**:
1. 磁盘上的已写字节数 + `buffer.len` == 逻辑写入总量 — *evidence: `storage.c:241`*
```

### Confidence 选择指南

- **high**: clause 直接可见于代码(显式校验 `if`/`assert`、显式赋值、显式 `return`)。
- **medium**: clause 由控制流/数据流推断(如:某个字段只在特定路径被设置,可以推断其范围)。
- **low**: clause 由领域惯例或设计模式隐含(如:"写入应当是原子的"来自存储系统惯例,代码中无显式实现)。**low confidence 的条款必须在 Phase 4 强制证伪。**

---

## 2. Property 模板

```markdown
### PROPERTY-{ID}: {short name}

**Type**: invariant | postcondition | metamorphic | model-based | inductive
**Applies to**: {CONTRACT-ID(s)}
**Statement**: ∀ inputs meeting {precondition}, {property holds}
**Confidence**: high | medium | low
**Evidence**: {code references}
```

### 范例 (每种 Type 各 1 个)

**invariant** (不变量保持):

```markdown
### PROPERTY-01: parse_byte_conservation

**Type**: invariant
**Applies to**: CONTRACT-03
**Statement**: ∀ valid inputs, `consumed_bytes + remaining_bytes == total_input_len`
**Confidence**: high
**Evidence**: `parser.c:90` explicitly assigns `remaining = len - consumed`
```

**postcondition** (后置条件):

```markdown
### PROPERTY-02: insert_queryable

**Type**: postcondition
**Applies to**: CONTRACT-05 (collection_insert)
**Statement**: ∀ element e, after `insert(e)` succeeds, `contains(e)` returns true
**Confidence**: high
**Evidence**: `collection.c:55` (insert adds to hash table), `collection.c:80` (contains checks hash table)
```

**metamorphic** (形变关系):

```markdown
### PROPERTY-03: sort_idempotent

**Type**: metamorphic
**Applies to**: CONTRACT-09 (sort_records)
**Statement**: ∀ input x, `sort(sort(x)) == sort(x)` — 排序是幂等的
**Confidence**: high
**Evidence**: `sort.c:120-150` implements comparison-based sort; already-sorted input produces identical output
```

**model-based** (基于模型):

```markdown
### PROPERTY-04: queue_matches_vecdeque

**Type**: model-based
**Applies to**: CONTRACT-06 (queue_dequeue)
**Statement**: ∀ input sequences, our queue's `dequeue()` produces same elements in same order as `VecDeque::pop_front()`
**Confidence**: medium
**Evidence**: `queue.c:30-50` implements FIFO with head/tail pointers matching VecDeque semantics
```

**inductive** (归纳):

```markdown
### PROPERTY-05: size_induction

**Type**: inductive
**Applies to**: CONTRACT-05 (collection_insert), CONTRACT-08 (collection_remove)
**Statement**: empty collection has `size() == 0`; each successful `insert` increases size by exactly 1; each successful `remove` decreases size by exactly 1
**Confidence**: high
**Evidence**: `collection.c:20` (init sets count=0), `collection.c:58` (insert increments count), `collection.c:95` (remove decrements count)
```

### Hughes 5-question 提示清单

1. **什么保持不变?** → invariant (操作前后什么量/关系不被改变)
2. **完成时保证什么?** → postcondition (操作成功后什么是真的)
3. **输入变换时输出如何变换?** → metamorphic (相同操作在不同但相关的输入之间的关系)
4. **有没有更简单的东西应该匹配?** → model-based (与参考实现/标准行为的等价性)
5. **有没有基本情况 + 归纳扩展?** → inductive (递归结构的基元和步骤)

---

## 3. State Model 模板

```markdown
## State Model: {system name}

**States**:
- **{S1}**: {description — 此状态下系统能做什么、不能做什么}
- **{S2}**: {description}
...

**Initial state**: {state}

**Transitions**:
| From | Event (triggering operation) | To | Guard (precondition) | Effect (postcondition) |
|------|-----|-----|-----|------|
| {S1} | {op_x()} | {S2} | {pre_clause} | {effect_clause} |

**Illegal transitions** (observed absent in code):
- ({S1}, {S3}) — {possible reason — 如:代码中无此路径,或 guard 永远不满足}

**Cross-state invariants**:
1. {something true in all states — 如:state 字段值 ∈ 已定义枚举}
```

### 范例: 网络连接状态模型

```markdown
## State Model: network_session

**States**:
- **INIT**: 已分配资源,尚未建立连接。不接受数据发送操作。
- **ACTIVE**: 连接已建立,可发送/接收数据。
- **RECONNECTING**: 连接中断,正在尝试重连。不接受新的发送操作。
- **CLOSED**: 连接已关闭,所有资源已释放。终态,不可逆转。

**Initial state**: INIT

**Transitions**:
| From | Event | To | Guard | Effect |
|------|-------|-----|-------|--------|
| INIT | `connect()` | ACTIVE | endpoint 有效 & 网络可达 | fd 被赋值,state=ACTIVE |
| INIT | `connect()` | INIT | endpoint 无效或网络不可达 | 返回 ERR_CONNECT,无副作用 |
| ACTIVE | `close()` | CLOSED | 总是允许 | fd 被释放,缓冲区清空 |
| ACTIVE | I/O error | RECONNECTING | 检测到连接异常 | retry_count=0,state=RECONNECTING |
| RECONNECTING | `reconnect()` | ACTIVE | 重连成功 | fd 重新赋值,state=ACTIVE |
| RECONNECTING | `reconnect()` | CLOSED | retry_count >= MAX_RETRIES | 所有资源释放 |

**Illegal transitions** (observed absent in code):
- (CLOSED, *) — 终态,代码中无任何从 CLOSED 出发的转换
- (INIT, CLOSED) — 无显式 close-from-init 路径,init 失败的连接不会到达 INIT

**Cross-state invariants**:
1. `state` 字段值 ∈ {INIT, ACTIVE, RECONNECTING, CLOSED}
2. CLOSED 状态下 `fd` 必为无效值 (-1)
```

---

## 4. Witness 模板 (Given/When/Then)

Witnesses 是契约或属性的**具体示例** — 它们不是主规格,而是帮助理解规格的具体化。每个 Witness 必须标注它所例证的 Contract 或 Property。

```markdown
### WITNESS-{ID}: {witness name}

**Witnesses**: {CONTRACT-ID or PROPERTY-ID}
**Source ref**: `{file}:{line}`

**Given** {pre-state reflecting the contract's preconditions}
- {condition}

**When** {action exercising the contract/property}
- {operation}

**Then** {observable outcome confirming the contract/property}
- {verifiable assertion with concrete values}
- {verifiable assertion with concrete values}

**Priority**: critical | high | medium | low
**Kind**: happy-path | error-path | boundary | edge-case | state-transition | format-assertion
```

**Witness Kind 说明**: Kind 现在仅用于 Witness,不用于 Contract/Property。

### 范例 (各 Kind 各 1 个)

**happy-path**:

```markdown
### WITNESS-01: parse_valid_data_message

**Witnesses**: CONTRACT-03
**Source ref**: `parser.c:40-95`

**Given** a valid byte buffer of 100 bytes with correct checksum header
**When** `parse_message(data, 100)` is called
**Then** return value is `Ok(Message)` with `header.type == DATA`
 And `consumed_bytes == 100`
 And `remaining_bytes == 0`

**Priority**: high  **Kind**: happy-path
```

**error-path**:

```markdown
### WITNESS-02: parse_checksum_failure

**Witnesses**: CONTRACT-03
**Source ref**: `parser.c:60-62`

**Given** a byte buffer with valid header but corrupted checksum field
**When** `parse_message(data, len)` is called
**Then** return value is `Err(ERR_CHECKSUM)`
 And no output Message fields are modified

**Priority**: high  **Kind**: error-path
```

**boundary**:

```markdown
### WITNESS-03: parse_max_size_message

**Witnesses**: CONTRACT-03
**Source ref**: `parser.c:44`

**Given** a valid byte buffer of exactly MAX_MSG_SIZE (65536) bytes
**When** `parse_message(data, 65536)` is called
**Then** return value is `Ok(Message)`
 And `payload.len == 65536 - HEADER_SIZE`

**Priority**: medium  **Kind**: boundary
```

**edge-case**:

```markdown
### WITNESS-04: parse_after_partial_read_recovery

**Witnesses**: PROPERTY-01
**Source ref**: `parser.c:90`

**Given** a previous parse consumed 50 of 100 bytes (partial read scenario)
**When** `parse_message(remaining_data, 50)` is called on the remaining bytes
**Then** `consumed_bytes + remaining_bytes == 50`

**Priority**: high  **Kind**: edge-case
```

**state-transition**:

```markdown
### WITNESS-05: session_connect_success

**Witnesses**: State Model (network_session), CONTRACT-07
**Source ref**: `session.c:50-75`

**Given** session is in state INIT with valid endpoint configuration
**When** `connect()` is called and endpoint is reachable
**Then** session transitions to state ACTIVE
 And `fd` is assigned a valid file descriptor
 And subsequent `send()` calls are accepted (no ERR_INVALID_STATE)

**Priority**: high  **Kind**: state-transition
```

**format-assertion**:

```markdown
### WITNESS-06: on_disk_magic_word

**Witnesses**: CONTRACT-12
**Source ref**: `storage.c:210`

**Given** a file has been written by the storage module
**When** the first 4 bytes are read from disk offset 0
**Then** bytes[0..4] == [0x46, 0x44, 0x42, 0x30]  (ASCII "FDB0", magic word)

**Priority**: critical  **Kind**: format-assertion
```

---

## 5. 反模式速查

### Then 断言关键词黑名单 (适用于 witnesses)

```
vague_keywords = {
    "正确", "合适", "合理", "良好", "恰当", "成功",
    "expected", "correct", "proper", "appropriate", "suitable",
    "as expected", "expected behavior",
}
```

任一行包含黑名单词 → 重写为具体值。

### Evidence 引用反模式 (适用于 contracts/properties)

| ❌ | ✅ |
|---|---|
| *(no evidence cited)* | *evidence: `parser.c:42`* |
| "based on general understanding" | *evidence: `Session::check()` performs this check before every transition* |
| "typical for this kind of code" | *evidence: `{file}:{line}`, pattern: {named design pattern}* |
| "implied by the design" | *evidence: `init.c:15-20` explicitly sets `count = 0`* |

### Contract vs Witness 反模式

| ❌ 反模式 | ✅ 正确做法 |
|---|---|
| 把"具体输入的具体值"写成 Contract 条款 | Contract 写通用条款;具体输入的具体值写成 Witness |
| 一个 Witness 没有对应的 Contract/Property | 每个 Witness 都必须标注 **Witnesses: {ID}** |
| Contract 没有 evidence 引用 | 每个条款必须有 evidence |
| Property 的 Statement 用"有时"/"偶尔"修饰 | Property 用 ∀ 或 ∃ 量化,明确声明范围 |
| 把 State transition 写成独立 Contract 而非放入 State Model | 状态转换属于 State Model,Contract 只声明 guard/effect |
| Witness 的 Then 用 "should" 而非确定的 "is"/"==" | Witness 描述确定的行为: `returns Ok(())` 而非 `should return Ok(())`|

---

## ID 命名规范

| 类型 | 前缀 | 示例 |
|---|---|---|
| Contract | `CONTRACT-{N}` | `CONTRACT-07` |
| Property | `PROPERTY-{N}` | `PROPERTY-12` |
| Witness | `WITNESS-{N}` | `WITNESS-03` |
| State transition | `TRANSITION-{N}` | `TRANSITION-02` |

ID 编号按模块/子系统连续编排,不按 Kind 分类。同一个 inflection point 的 Contract、Properties 和 Witnesses 共享相似的编号区间,便于交叉引用。
