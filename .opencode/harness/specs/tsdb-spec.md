# 验收规格: TSDB 时序数据库

> 涵盖 Feature 5, 6, 7, 8 | 优先级: critical
> 源 BL 文档: bl/tsdb-*.md

## User Story

作为嵌入式应用开发者，
我想要使用时间序列数据库按时间顺序存储和查询日志数据，
以便记录传感器采集数据和设备运行日志，并支持按时间范围高效查询。

---

## Scenarios

### Happy Path

#### Scenario: 首次初始化空的 TSDB 实例

**Given** 一个未初始化的 TSDB 实例，存储分区为空（全 0xFF），提供了 `get_time` 回调
**When** 调用 `fdb_tsdb_init(db, "logdb", "fdb_tsdb1", get_time, 256, NULL)` 初始化
**Then** 返回值等于 `FDB_NO_ERR`
**And** 实例的 `init_ok` 标志为 true
**And** `rollover` 默认为 true
**And** `last_time` 为 0

#### Scenario: 追加单条 TSL（自动时间戳）

**Given** 一个已初始化的 TSDB 实例，`get_time` 返回时间戳 1000
**When** 调用 `fdb_tsl_append(db, blob)` 追加 64 字节数据
**Then** 返回值等于 `FDB_NO_ERR`
**And** `db->last_time` 更新为 1000
**And** 当前扇区状态从 EMPTY 转为 USING

#### Scenario: 追加单条 TSL（指定时间戳）

**Given** 一个已初始化的 TSDB 实例，`last_time` 为 500
**When** 调用 `fdb_tsl_append_with_ts(db, blob, 600)` 追加 64 字节数据
**Then** 返回值等于 `FDB_NO_ERR`
**And** `db->last_time` 更新为 600

#### Scenario: 正向遍历所有 TSL

**Given** 一个已初始化的 TSDB 实例，包含 5 条 TSL，时间戳分别为 100、200、300、400、500
**When** 调用 `fdb_tsl_iter(db, cb, arg)` 正向遍历
**Then** 回调 `cb` 按时间戳 100→200→300→400→500 的顺序被调用 5 次

#### Scenario: 反向遍历所有 TSL

**Given** 一个已初始化的 TSDB 实例，包含 5 条 TSL，时间戳分别为 100、200、300、400、500
**When** 调用 `fdb_tsl_iter_reverse(db, cb, arg)` 反向遍历
**Then** 回调 `cb` 按时间戳 500→400→300→200→100 的顺序被调用 5 次

#### Scenario: 按时间范围正向查询

**Given** 一个已初始化的 TSDB 实例，包含时间戳为 100、200、300、400、500 的 5 条 TSL
**When** 调用 `fdb_tsl_iter_by_time(db, 200, 400, cb, arg)` 查询时间范围 [200, 400]
**Then** 回调 `cb` 按时间戳 200→300→400 的顺序被调用 3 次

#### Scenario: 按时间范围反向查询（from > to）

**Given** 一个已初始化的 TSDB 实例，包含时间戳为 100、200、300、400、500 的 5 条 TSL
**When** 调用 `fdb_tsl_iter_by_time(db, 400, 200, cb, arg)` 查询（from=400 > to=200）
**Then** 回调 `cb` 按时间戳 400→300→200 的顺序被调用 3 次（反向遍历）

#### Scenario: 统计指定状态的 TSL 数量

**Given** 一个已初始化的 TSDB 实例，时间范围 [100, 500] 内有 5 条 TSL，其中 3 条状态为 `FDB_TSL_WRITE`
**When** 调用 `fdb_tsl_query_count(db, 100, 500, FDB_TSL_WRITE)`
**Then** 返回值为 3

#### Scenario: 修改 TSL 状态

**Given** 一个已初始化的 TSDB 实例，有一条 TSL 状态为 `FDB_TSL_WRITE`
**When** 调用 `fdb_tsl_set_status(db, &tsl, FDB_TSL_DELETED)` 修改状态
**Then** 返回值等于 `FDB_NO_ERR`
**And** 重新读取该 TSL，状态为 `FDB_TSL_DELETED`

#### Scenario: 回调终止遍历

**Given** 一个已初始化的 TSDB 实例，包含 5 条 TSL
**When** 调用 `fdb_tsl_iter`，回调 `cb` 在第 3 条时返回 true
**Then** 遍历在第 3 条后终止
**And** 回调总共被调用 3 次

#### Scenario: 清空数据库

**Given** 一个已初始化的 TSDB 实例，包含多条 TSL
**When** 调用 `fdb_tsl_clean(db)` 清空
**Then** 所有扇区被格式化为 EMPTY 状态
**And** `last_time` 重置为 0
**And** 后续遍历不产出任何 TSL

### Sad Path

#### Scenario: 初始化时未提供 get_time 回调

**Given** 一个 TSDB 实例，`get_time` 参数为 NULL
**When** 调用 `fdb_tsdb_init(db, "logdb", "part", NULL, 256, NULL)`
**Then** 触发 FDB_ASSERT 断言失败

#### Scenario: max_len 大于等于扇区大小

**Given** 一个 TSDB 实例，`sec_size = 4096`
**When** 调用 `fdb_tsdb_init(db, "logdb", "part", get_time, 4096, NULL)`（max_len == sec_size）
**Then** 触发 FDB_ASSERT 断言失败

#### Scenario: 追加时时间戳非单调递增

**Given** 一个已初始化的 TSDB 实例，`last_time` 为 500
**When** 调用 `fdb_tsl_append_with_ts(db, blob, 400)`（时间戳 < last_time）
**Then** 返回值等于 `FDB_WRITE_ERR`
**And** 该 TSL 被丢弃

#### Scenario: 追加时时间戳等于 last_time

**Given** 一个已初始化的 TSDB 实例，`last_time` 为 500
**When** 调用 `fdb_tsl_append_with_ts(db, blob, 500)`（时间戳 == last_time）
**Then** 返回值等于 `FDB_WRITE_ERR`（严格大于，等于也拒绝）

#### Scenario: 追加时 blob 大小超过 max_len

**Given** 一个已初始化的 TSDB 实例，`max_len = 256`
**When** 调用 `fdb_tsl_append` 追加 300 字节数据
**Then** 返回值等于 `FDB_WRITE_ERR`

#### Scenario: 固定 blob 模式大小不匹配

**Given** 一个已初始化的 TSDB 实例，定义了 `FDB_TSDB_FIXED_BLOB_SIZE = 4`
**When** 调用 `fdb_tsl_append` 追加 8 字节数据
**Then** 返回值等于 `FDB_WRITE_ERR`

#### Scenario: rollover=false 且数据库已满

**Given** 一个已初始化的 TSDB 实例，`rollover = false`，所有扇区已满
**When** 调用 `fdb_tsl_append` 追加新数据
**Then** 返回值等于 `FDB_SAVED_FULL`

#### Scenario: 未初始化时调用追加

**Given** 一个未初始化的 TSDB 实例（`init_ok == false`）
**When** 调用 `fdb_tsl_append(db, blob)`
**Then** 返回值等于 `FDB_INIT_FAILED`

#### Scenario: 初始化时多个扇区处于 USING 状态

**Given** 一个 TSDB 实例，存储分区中有 2 个扇区的状态均为 USING
**When** 调用 `fdb_tsdb_init`
**Then** 触发全量格式化（`tsl_format_all`）（`not_formatable == false` 时）
**And** 返回值等于 `FDB_NO_ERR`

### Edge Cases

#### Scenario: 扇区满后自动切换到下一扇区

**Given** 一个已初始化的 TSDB 实例，当前扇区剩余空间不足容纳新 TSL，非最后一个扇区
**When** 调用 `fdb_tsl_append` 追加新数据
**Then** 返回值等于 `FDB_NO_ERR`
**And** 当前扇区状态转为 FULL
**And** 下一扇区被格式化为 USING 状态
**And** `oldest_addr` 被更新

#### Scenario: 最后一个扇区满后环形回绕（rollover=true）

**Given** 一个已初始化的 TSDB 实例，`rollover = true`，当前使用最后一个扇区且空间不足
**When** 调用 `fdb_tsl_append`
**Then** 返回值等于 `FDB_NO_ERR`
**And` 扇区 0 被格式化并作为新的当前扇区

#### Scenario: 掉电恢复 — TSL 写入中断

**Given** 一个 TSDB 实例，上次运行时 TSL 索引写入后（PRE_WRITE 状态）、数据写入前掉电
**When** 重新调用 `fdb_tsdb_init`
**Then** 返回值等于 `FDB_NO_ERR`
**And** 中断的 TSL 被视为 UNUSED（`read_tsl` 返回 `log_len = max_len`, `time = 0`）

#### Scenario: 掉电恢复 — 扇区头部损坏

**Given** 一个 TSDB 实例，部分扇区头部 magic word 不正确
**When** 调用 `fdb_tsdb_init`（`not_formatable == false`）
**Then** 执行全量格式化（`tsl_format_all`）
**And** 返回值等于 `FDB_NO_ERR`

#### Scenario: not_formatable 模式下头部损坏

**Given** 一个 TSDB 实例，部分扇区头部损坏，`not_formatable == true`
**When** 调用 `fdb_tsdb_init`
**Then** 返回值等于 `FDB_READ_ERR`

#### Scenario: SET_ROLLOVER 控制命令（初始化后）

**Given** 一个已初始化的 TSDB 实例（`init_ok == true`），`rollover = true`
**When** 调用 `fdb_tsdb_control(db, FDB_TSDB_CTRL_SET_ROLLOVER, &false_val)` 设置 rollover 为 false
**Then** `db->rollover` 变为 false
**And** 后续空间耗尽时追加返回 `FDB_SAVED_FULL`

#### Scenario: SET_SEC_SIZE 控制命令（初始化前）

**Given** 一个未初始化的 TSDB 实例（`init_ok == false`）
**When** 调用 `fdb_tsdb_control(db, FDB_TSDB_CTRL_SET_SEC_SIZE, &size)` 设置扇区大小
**Then** `db->sec_size` 被设置为指定值
**And** 后续初始化使用该扇区大小

#### Scenario: SET_ROLLOVER 在初始化前调用

**Given** 一个未初始化的 TSDB 实例（`init_ok == false`）
**When** 调用 `fdb_tsdb_control(db, FDB_TSDB_CTRL_SET_ROLLOVER, &true_val)`
**Then** 触发 FDB_ASSERT 断言失败（SET_ROLLOVER 必须在初始化后）

#### Scenario: 空数据库遍历

**Given** 一个已初始化的空 TSDB 实例（所有扇区为 EMPTY）
**When** 调用 `fdb_tsl_iter(db, cb, arg)`
**Then** 回调 `cb` 不被调用
**And** 遍历立即返回

#### Scenario: 首条 TSL 追加（last_time=0）

**Given** 一个已初始化的 TSDB 实例，`last_time = 0`（首次或 clean 后）
**When** 调用 `fdb_tsl_append_with_ts(db, blob, 1)` 追加时间戳为 1 的 TSL
**Then** 返回值等于 `FDB_NO_ERR`（1 > 0 满足单调性）

#### Scenario: 查询最大 TSL 容量

**Given** 一个已初始化的 TSDB 实例，`sec_size = 4096`，`max_size = 8192`（2 个扇区），`max_len = 64`
**When** 调用 `fdb_tsl_max_blob_count(db)`
**Then** 返回值为 2 * ((4096 - SECTOR_HDR_DATA_SIZE) / (LOG_IDX_DATA_SIZE + FDB_WG_ALIGN(64)))

---

## Acceptance Criteria

1. `fdb_tsdb_init` 在空分区上初始化后，`init_ok` 为 true，`rollover` 为 true
2. `fdb_tsl_append` 后 `last_time` 更新为追加的时间戳
3. `fdb_tsl_iter` 按时间戳升序调用回调
4. `fdb_tsl_iter_reverse` 按时间戳降序调用回调
5. `fdb_tsl_iter_by_time(from, to)` 当 `from <= to` 时正向遍历，当 `from > to` 时反向遍历
6. `fdb_tsl_query_count` 返回指定时间范围和状态的 TSL 数量
7. `fdb_tsl_set_status` 修改 TSL 状态后，重新读取状态一致
8. `fdb_tsl_clean` 后所有扇区为 EMPTY，`last_time` 为 0
9. `get_time` 为 NULL 时触发断言
10. `max_len >= sec_size` 时触发断言
11. 时间戳 <= `last_time` 时追加返回 `FDB_WRITE_ERR`
12. blob 大小 > `max_len` 时追加返回 `FDB_WRITE_ERR`
13. `rollover = false` 且数据库满时追加返回 `FDB_SAVED_FULL`
14. 扇区满后自动切换到下一扇区（rollover=true 时环形回绕）
15. 掉电恢复后，PRE_WRITE 状态的 TSL 被视为 UNUSED
16. `SET_ROLLOVER` 必须在初始化后调用，否则触发断言

---

## Non-Functional Requirements

- **性能**: 追加 TSL 平均 4ms（NOR Flash W25Q64），查询平均 1.8ms/TSL
- **可靠性**: 掉电后通过两阶段写入（PRE_WRITE → WRITE）保证数据一致性
- **资源占用**: RAM 占用接近 0，代码体积 < 1.5KB（IAR 优化）
- **存储模型**: 索引从扇区头向下增长，数据从扇区底向上增长，空间利用率最大化
