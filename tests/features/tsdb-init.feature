@tsdb @init @config
Feature: TSDB 时序数据库初始化与配置

  Background:
    Given FlashDB 库已编译并链接到测试程序
    And 一个可用的 Flash 存储后端

  Scenario: 首次初始化空分区返回成功
    Given Flash 分区 "fdb_tsdb1" 为空（全 0xFF）
    And get_time 回调函数返回当前时间戳
    When 调用 fdb_tsdb_init(db, "logdb", "fdb_tsdb1", get_time, 256, NULL) 初始化
    Then 返回值等于 FDB_NO_ERR
    And 实例的 rollover 为 true
    And 实例的 last_time 为 0

  Scenario: 初始化时未提供 get_time 回调触发断言
    Given get_time 参数为 NULL
    When 调用 fdb_tsdb_init(db, "logdb", "part", NULL, 256, NULL)
    Then 触发 FDB_ASSERT 断言失败

  Scenario: max_len 大于等于扇区大小触发断言
    Given 扇区大小为 4096 字节
    When 调用 fdb_tsdb_init(db, "logdb", "part", get_time, 4096, NULL)
    Then 触发 FDB_ASSERT 断言失败

  Scenario: 多个 USING 扇区触发全量格式化
    Given Flash 分区中有 2 个扇区状态均为 USING
    And not_formatable 为 false
    When 调用 fdb_tsdb_init 初始化
    Then 返回值等于 FDB_NO_ERR
    And 所有扇区被格式化为 EMPTY 状态

  Scenario: 扇区头部损坏且可格式化时全量恢复
    Given Flash 分区部分扇区 magic word 被破坏
    And not_formatable 为 false
    When 调用 fdb_tsdb_init 初始化
    Then 返回值等于 FDB_NO_ERR
    And 所有扇区被格式化为 EMPTY 状态

  Scenario: 扇区头部损坏且 not_formatable 返回读取错误
    Given Flash 分区部分扇区 magic word 被破坏
    And not_formatable 为 true
    When 调用 fdb_tsdb_init 初始化
    Then 返回值等于 FDB_READ_ERR

  Scenario: SET_ROLLOVER 在初始化后修改 rollover 行为
    Given TSDB 实例已初始化，rollover 为 true
    When 调用 fdb_tsdb_control(db, FDB_TSDB_CTRL_SET_ROLLOVER, &false_val)
    Then db 的 rollover 为 false
    And 后续空间耗尽时追加返回 FDB_SAVED_FULL

  Scenario: SET_ROLLOVER 在初始化前调用触发断言
    Given TSDB 实例未初始化（init_ok 为 false）
    When 调用 fdb_tsdb_control(db, FDB_TSDB_CTRL_SET_ROLLOVER, &true_val)
    Then 触发 FDB_ASSERT 断言失败

  Scenario Outline: 初始化前可设置 <配置项>
    Given TSDB 实例未初始化（init_ok 为 false）
    When 调用 fdb_tsdb_control 设置 <配置项>
    Then 配置项被设置且不触发断言

    Examples:
      | 配置项                    |
      | FDB_TSDB_CTRL_SET_SEC_SIZE |
      | FDB_TSDB_CTRL_SET_FILE_MODE |
      | FDB_TSDB_CTRL_SET_MAX_SIZE |
      | FDB_TSDB_CTRL_SET_NOT_FORMAT |

  Scenario: GET_LAST_TIME 获取最后保存时间戳
    Given TSDB 实例已初始化，last_time 为 500
    When 调用 fdb_tsdb_control(db, FDB_TSDB_CTRL_GET_LAST_TIME, &time)
    Then time 的值为 500

  Scenario: 反初始化后 init_ok 为 false
    Given TSDB 实例已初始化
    When 调用 fdb_tsdb_deinit(db)
    Then 实例的 init_ok 为 false
