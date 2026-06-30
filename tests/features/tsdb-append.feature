@tsdb @append
Feature: TSDB 时序数据追加

  Background:
    Given TSDB 实例已初始化，名称为 "logdb"，max_len 为 256，rollover 为 true
    And get_time 回调返回单调递增时间戳

  Scenario: 追加单条 TSL（自动时间戳）返回成功
    Given get_time 返回时间戳 1000
    When 调用 fdb_tsl_append(db, blob) 追加 64 字节数据
    Then 返回值等于 FDB_NO_ERR
    And db 的 last_time 为 1000
    And 当前扇区状态为 USING

  Scenario: 追加单条 TSL（指定时间戳）返回成功
    Given db 的 last_time 为 500
    When 调用 fdb_tsl_append_with_ts(db, blob, 600) 追加 64 字节数据
    Then 返回值等于 FDB_NO_ERR
    And db 的 last_time 为 600

  Scenario: 首条 TSL 时间戳只需大于 0
    Given db 的 last_time 为 0（首次或 clean 后）
    When 调用 fdb_tsl_append_with_ts(db, blob, 1) 追加
    Then 返回值等于 FDB_NO_ERR

  Scenario Outline: 追加时时间戳 <关系> last_time 返回写入失败
    Given db 的 last_time 为 500
    When 调用 fdb_tsl_append_with_ts(db, blob, <ts>) 追加
    Then 返回值等于 FDB_WRITE_ERR

    Examples:
      | 关系 | ts  |
      | 小于 | 400 |
      | 等于 | 500 |

  Scenario: 追加时 blob 大小超过 max_len 返回写入失败
    When 调用 fdb_tsl_append 追加 300 字节数据
    Then 返回值等于 FDB_WRITE_ERR

  Scenario: 固定 blob 模式大小不匹配返回写入失败
    Given FDB_TSDB_FIXED_BLOB_SIZE 定义为 4
    When 调用 fdb_tsl_append 追加 8 字节数据
    Then 返回值等于 FDB_WRITE_ERR

  Scenario: 未初始化时追加返回初始化失败
    Given TSDB 实例未初始化（init_ok 为 false）
    When 调用 fdb_tsl_append(db, blob)
    Then 返回值等于 FDB_INIT_FAILED

  Scenario: 扇区满后自动切换到下一扇区
    Given 当前扇区剩余空间不足容纳新 TSL
    And 当前扇区非最后一个扇区
    When 调用 fdb_tsl_append 追加新数据
    Then 返回值等于 FDB_NO_ERR
    And 当前扇区状态转为 FULL
    And 下一扇区状态转为 USING

  Scenario: 最后一个扇区满后环形回绕
    Given rollover 为 true
    And 当前使用最后一个扇区且空间不足
    When 调用 fdb_tsl_append
    Then 返回值等于 FDB_NO_ERR
    And 扇区 0 被格式化并作为新的当前扇区

  Scenario: rollover 为 false 且最后扇区满返回空间已满
    Given rollover 为 false
    And 当前使用最后一个扇区且空间不足
    When 调用 fdb_tsl_append
    Then 返回值等于 FDB_SAVED_FULL

  Scenario: 下一扇区非空时自动格式化
    Given 当前扇区空间不足，下一扇区状态为 FULL（非 EMPTY）
    And rollover 为 true
    When 调用 fdb_tsl_append
    Then 返回值等于 FDB_NO_ERR
    And 下一扇区被格式化为 USING 状态
    And oldest_addr 被更新

  Scenario: 掉电恢复时 PRE_WRITE 状态的 TSL 被视为未使用
    Given 上次运行时 TSL 索引写入后（PRE_WRITE 状态）掉电
    When 重新调用 fdb_tsdb_init
    Then 返回值等于 FDB_NO_ERR
    And 遍历该扇区时中断的 TSL 被视为 UNUSED（time 为 0，log_len 为 max_len）

  Scenario: 追加后读取 TSL 数据与写入一致
    When 调用 fdb_tsl_append_with_ts(db, blob, 100) 追加 64 字节数据
    And 调用 fdb_tsl_iter 遍历获取该 TSL
    And 调用 fdb_tsl_to_blob 转换为 blob
    And 调用 fdb_blob_read 读取
    Then 读取的 64 字节数据与写入的完全一致
