@kvdb @init @lifecycle
Feature: KVDB 数据库初始化与生命周期管理

  Background:
    Given FlashDB 库已编译并链接到测试程序
    And 一个可用的 Flash 存储后端（FAL 分区或文件模式）

  Scenario: 首次初始化空分区返回成功并加载默认 KV
    Given Flash 分区 "fdb_kvdb1" 为空（全 0xFF）
    And 默认 KV 集合包含键 "hostname" 值 "sensor-01"
    When 调用 fdb_kvdb_init 初始化 KVDB 实例，名称为 "config"，分区为 "fdb_kvdb1"
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get(db, "hostname") 返回字符串 "sensor-01"

  Scenario: 重复初始化已初始化的实例是幂等的
    Given KVDB 实例已初始化（init_ok 为 true）
    When 再次调用 fdb_kvdb_init
    Then 返回值等于 FDB_NO_ERR

  Scenario Outline: 初始化时违反 <约束> 返回 <错误码>
    Given KVDB 实例的 <约束> 被违反
    When 调用 fdb_kvdb_init
    Then 返回值等于 <错误码>

    Examples:
      | 约束                    | 错误码              |
      | 分区不存在              | FDB_PART_NOT_FOUND  |
      | 总大小非扇区整数倍      | FDB_INIT_FAILED     |
      | 扇区数不足 2 个         | FDB_INIT_FAILED     |

  Scenario: 所有扇区头部损坏时自动重置为默认值
    Given Flash 分区所有扇区的 magic word 均被破坏
    And not_formatable 为 false
    When 调用 fdb_kvdb_init 初始化实例
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get(db, "hostname") 返回默认值 "sensor-01"

  Scenario: not_formatable 模式下扇区损坏返回读取错误
    Given Flash 分区部分扇区 magic word 被破坏
    And not_formatable 为 true
    When 调用 fdb_kvdb_init 初始化实例
    Then 返回值等于 FDB_READ_ERR

  Scenario: 掉电恢复时 PRE_WRITE 状态的 KV 被标记为错误头部
    Given 上次运行时写入 KV 头部后掉电，该 KV 状态为 PRE_WRITE
    When 重新调用 fdb_kvdb_init
    Then 返回值等于 FDB_NO_ERR
    And 遍历所有 KV 时不产出该中断的 KV

  Scenario: 掉电恢复时 PRE_DELETE 状态的 KV 被恢复
    Given 上次运行时旧 KV 标记为 PRE_DELETE 后掉电
    When 重新调用 fdb_kvdb_init
    Then 返回值等于 FDB_NO_ERR
    And 该 KV 的旧值被恢复（可通过 fdb_kv_get 读取到旧值）

  Scenario: 掉电恢复时中断的 GC 被自动完成
    Given 上次运行时 GC 过程中掉电，某扇区 dirty 状态为 GC
    When 重新调用 fdb_kvdb_init
    Then 返回值等于 FDB_NO_ERR
    And 该扇区的 GC 被自动完成（有效 KV 被搬运，旧扇区被格式化）

  Scenario: 完整性检查通过已初始化的数据库
    Given KVDB 实例已初始化且数据完整
    When 调用 fdb_kvdb_check(db)
    Then 返回值等于 FDB_NO_ERR

  Scenario: 反初始化后 init_ok 为 false
    Given KVDB 实例已初始化
    When 调用 fdb_kvdb_deinit(db)
    Then 实例的 init_ok 为 false
