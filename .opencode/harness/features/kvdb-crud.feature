@kvdb @crud
Feature: KVDB 键值对写入与读取

  Background:
    Given KVDB 实例已初始化，名称为 "config"，扇区大小为 4096 字节

  Scenario: 写入并读取字符串 KV
    When 调用 fdb_kv_set(db, "hostname", "sensor-01") 写入字符串
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get(db, "hostname") 返回字符串 "sensor-01"

  Scenario: 写入并读取 Blob KV
    Given 准备一个 32 字节的 blob，内容为 0x01 到 0x20
    When 调用 fdb_kv_set_blob(db, "sensor_config", blob) 写入
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get_blob(db, "sensor_config", blob) 返回 32
    And blob 内容与写入的 0x01 到 0x20 完全一致

  Scenario: 更新已有 KV 的值后读取返回新值
    Given 键 "hostname" 的值为 "old-name"
    When 调用 fdb_kv_set(db, "hostname", "new-name") 更新
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get(db, "hostname") 返回字符串 "new-name"

  Scenario: 删除已有 KV 后读取返回未找到
    Given 键 "temp_key" 已存在
    When 调用 fdb_kv_del(db, "temp_key") 删除
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get_blob(db, "temp_key", blob) 返回 0

  Scenario: 读取二进制数据时 fdb_kv_get 返回 NULL
    Given 键 "bin_data" 存储了包含不可打印字符的二进制数据
    When 调用 fdb_kv_get(db, "bin_data")
    Then 返回值为 NULL

  Scenario Outline: 写入超长 key 名返回名称错误
    When 调用 fdb_kv_set 写入 key 名长度为 <key_len> 的 KV
    Then 返回值等于 <错误码>

    Examples:
      | key_len | 错误码            |
      | 64      | FDB_NO_ERR        |
      | 65      | FDB_KV_NAME_ERR   |
      | 128     | FDB_KV_NAME_ERR   |

  Scenario: 写入超大 KV（超过扇区容量）返回空间已满
    Given 扇区大小为 4096 字节
    When 调用 fdb_kv_set_blob 写入总长度超过扇区容量的 KV
    Then 返回值等于 FDB_SAVED_FULL

  Scenario: 删除不存在的 key 返回名称错误
    Given 键 "missing_key" 不存在
    When 调用 fdb_kv_del(db, "missing_key")
    Then 返回值等于 FDB_KV_NAME_ERR

  Scenario Outline: 未初始化时调用 <操作> 返回初始化失败
    Given KVDB 实例未初始化（init_ok 为 false）
    When 调用 <操作>
    Then 返回值等于 FDB_INIT_FAILED

    Examples:
      | 操作                          |
      | fdb_kv_set(db, "k", "v")     |
      | fdb_kv_get_blob(db, "k", b)  |
      | fdb_kv_del(db, "k")          |
      | fdb_kv_get_obj(db, "k", kv)  |

  Scenario: set 传 NULL 值等同于删除
    Given 键 "to_delete" 已存在
    When 调用 fdb_kv_set(db, "to_delete", NULL)
    Then 返回值等于 FDB_NO_ERR
    And 调用 fdb_kv_get_blob(db, "to_delete", blob) 返回 0

  Scenario: 获取 KV 对象后转换为 blob 读取值
    Given 键 "config" 的值为 16 字节二进制数据
    When 调用 fdb_kv_get_obj(db, "config", &kv) 获取对象
    And 调用 fdb_kv_to_blob(&kv, &blob) 转换
    And 调用 fdb_blob_read(db, &blob) 读取
    Then blob 数据与原始 16 字节完全一致
