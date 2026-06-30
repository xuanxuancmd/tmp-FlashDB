@kvdb @iteration @gc
Feature: KVDB 键值迭代遍历与垃圾回收

  Background:
    Given KVDB 实例已初始化，名称为 "config"

  Scenario: 迭代遍历产出所有有效 KV
    Given 数据库包含 3 个有效 KV（状态为 FDB_KV_WRITE 且 CRC 通过）
    And 数据库包含 1 个已删除 KV（状态为 FDB_KV_DELETED）
    When 调用 fdb_kv_iterator_init 初始化迭代器
    And 循环调用 fdb_kv_iterate 直到返回 false
    Then 迭代器产出 3 个 KV
    And 迭代器统计 iterated_cnt 等于 3

  Scenario: 迭代器统计累计字节数
    Given 数据库包含 2 个 KV，value 长度分别为 60 和 100 字节
    When 调用 fdb_kv_iterator_init 初始化迭代器
    And 循环调用 fdb_kv_iterate 直到返回 false
    Then 迭代器统计 iterated_value_bytes 等于 160

  Scenario: 空数据库迭代立即返回 false
    Given 数据库为空（无有效 KV）
    When 调用 fdb_kv_iterator_init 初始化迭代器
    And 调用 fdb_kv_iterate
    Then 返回值为 false

  Scenario: 打印所有 KV 输出键值对
    Given 数据库包含字符串 KV "hostname=sensor-01"
    When 调用 fdb_kv_print(db)
    Then 标准输出包含 "hostname=sensor-01"

  Scenario: GC 回收已删除 KV 的空间
    Given 扇区 A 包含 3 个 KV，其中 2 个已标记为 DELETED
    And 空闲扇区数不足触发 GC
    When GC 被触发执行
    Then 扇区 A 中的 1 个有效 KV 被搬运到其他扇区
    And 扇区 A 被格式化为 EMPTY 状态
    And 迭代遍历仍能找到那个有效 KV

  Scenario: GC 搬运后 KV 值保持不变
    Given 键 "important" 的值为 64 字节二进制数据
    And 该 KV 所在扇区触发 GC 搬运
    When GC 完成后调用 fdb_kv_get_blob(db, "important", blob)
    Then 返回 64
    And blob 内容与搬运前完全一致

  Scenario: GC 后已删除 KV 不再出现
    Given 键 "deleted_key" 已标记为 DELETED
    When 该 KV 所在扇区触发 GC
    Then GC 完成后迭代遍历不产出 "deleted_key"
    And 调用 fdb_kv_get_blob(db, "deleted_key", blob) 返回 0

  Scenario: 数据库空间耗尽且无可回收垃圾返回空间已满
    Given 所有扇区已满且所有 KV 均为有效状态
    When 调用 fdb_kv_set_blob 写入新 KV
    Then 返回值等于 FDB_SAVED_FULL

  Scenario: 写入触发扇区变满后标记 GC 请求
    Given 当前扇区剩余空间仅够写入 1 个 KV
    When 调用 fdb_kv_set_blob 写入 1 个 KV
    Then 返回值等于 FDB_NO_ERR
    And db 的 gc_request 为 true
