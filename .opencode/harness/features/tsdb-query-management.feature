@tsdb @query @management
Feature: TSDB 时序数据查询与管理

  Background:
    Given TSDB 实例已初始化，名称为 "logdb"
    And 数据库包含 5 条 TSL，时间戳分别为 100、200、300、400、500

  Scenario: 正向遍历按时间升序调用回调
    When 调用 fdb_tsl_iter(db, cb, arg) 正向遍历
    Then 回调 cb 按时间戳 100、200、300、400、500 的顺序被调用 5 次

  Scenario: 反向遍历按时间降序调用回调
    When 调用 fdb_tsl_iter_reverse(db, cb, arg) 反向遍历
    Then 回调 cb 按时间戳 500、400、300、200、100 的顺序被调用 5 次

  Scenario: 按时间范围正向查询
    When 调用 fdb_tsl_iter_by_time(db, 200, 400, cb, arg) 查询范围 [200, 400]
    Then 回调 cb 按时间戳 200、300、400 的顺序被调用 3 次

  Scenario: 按时间范围反向查询（from 大于 to）
    When 调用 fdb_tsl_iter_by_time(db, 400, 200, cb, arg) 查询
    Then 回调 cb 按时间戳 400、300、200 的顺序被调用 3 次（反向遍历）

  Scenario: 回调返回 true 终止遍历
    Given 回调 cb 在第 3 条 TSL 时返回 true
    When 调用 fdb_tsl_iter(db, cb, arg)
    Then 回调 cb 总共被调用 3 次

  Scenario: 统计指定状态的 TSL 数量
    Given 时间范围 [100, 500] 内有 3 条 TSL 状态为 FDB_TSL_WRITE
    When 调用 fdb_tsl_query_count(db, 100, 500, FDB_TSL_WRITE)
    Then 返回值为 3

  Scenario: 查询空数据库的 TSL 数量返回 0
    Given 数据库为空（所有扇区为 EMPTY）
    When 调用 fdb_tsl_query_count(db, 100, 500, FDB_TSL_WRITE)
    Then 返回值为 0

  Scenario: 查询最大 TSL 容量
    Given sec_size 为 4096，max_size 为 8192（2 个扇区），max_len 为 64
    When 调用 fdb_tsl_max_blob_count(db)
    Then 返回值为 100

  Scenario: 修改 TSL 状态为已删除
    Given 有一条 TSL 状态为 FDB_TSL_WRITE
    When 调用 fdb_tsl_set_status(db, &tsl, FDB_TSL_DELETED)
    Then 返回值等于 FDB_NO_ERR
    And 重新读取该 TSL 状态为 FDB_TSL_DELETED

  Scenario: 清空数据库后所有扇区为 EMPTY
    Given 数据库包含多条 TSL
    When 调用 fdb_tsl_clean(db)
    Then 所有扇区被格式化为 EMPTY 状态
    And db 的 last_time 为 0
    And 后续调用 fdb_tsl_iter 不产出任何 TSL

  Scenario: 空数据库正向遍历不调用回调
    Given 数据库为空（所有扇区为 EMPTY）
    When 调用 fdb_tsl_iter(db, cb, arg)
    Then 回调 cb 不被调用

  Scenario: 遍历到 EMPTY 扇区时终止
    Given 数据库中部分扇区为 USING/FULL，后续扇区为 EMPTY
    When 调用 fdb_tsl_iter 正向遍历
    Then 遇到第一个 EMPTY 扇区时遍历终止
    And 回调仅对 USING/FULL 扇区中的 TSL 被调用
