# 验收规格: KVDB 键值数据库

> 涵盖 Feature 1, 2, 3, 4, 11 | 优先级: critical
> 源 BL 文档: bl/kvdb-*.md

## User Story

作为嵌入式应用开发者，
我想要使用键值数据库存储和读取配置参数，
以便在设备重启后仍能持久化配置数据，且在掉电时保证数据一致性。

---

## Scenarios

### Happy Path

#### Scenario: 首次初始化空的 KVDB 实例

**Given** 一个未初始化的 KVDB 实例，存储分区为空（全 0xFF）
**When** 调用 `fdb_kvdb_init` 初始化实例，提供名称 "config"、分区名 "fdb_kvdb1"、默认 KV 集合
**Then** 返回值等于 `FDB_NO_ERR`
**And** 实例的 `init_ok` 标志为 true
**And** 所有默认 KV 可通过 `fdb_kv_get_blob` 读取到对应值

#### Scenario: 写入并读取字符串 KV

**Given** 一个已初始化的 KVDB 实例
**When** 调用 `fdb_kv_set(db, "hostname", "sensor-01")` 写入字符串
**Then** 返回值等于 `FDB_NO_ERR`
**And** 随后调用 `fdb_kv_get(db, "hostname")` 返回字符串 "sensor-01"

#### Scenario: 写入并读取 Blob KV

**Given** 一个已初始化的 KVDB 实例
**When** 调用 `fdb_kv_set_blob` 写入 32 字节的二进制数据到键 "sensor_config"
**Then** 返回值等于 `FDB_NO_ERR`
**And** 随后调用 `fdb_kv_get_blob` 读取 "sensor_config" 返回 32 字节
**And** 读取的数据与写入的数据完全一致

#### Scenario: 更新已有 KV 的值

**Given** 一个已初始化的 KVDB 实例，键 "hostname" 的值为 "old-name"
**When** 调用 `fdb_kv_set(db, "hostname", "new-name")` 更新值
**Then** 返回值等于 `FDB_NO_ERR`
**And** 随后调用 `fdb_kv_get(db, "hostname")` 返回字符串 "new-name"

#### Scenario: 删除已有 KV

**Given** 一个已初始化的 KVDB 实例，键 "temp_key" 已存在
**When** 调用 `fdb_kv_del(db, "temp_key")` 删除
**Then** 返回值等于 `FDB_NO_ERR`
**And** 随后调用 `fdb_kv_get_blob(db, "temp_key", &blob)` 返回 0（未找到）

#### Scenario: 迭代遍历所有 KV

**Given** 一个已初始化的 KVDB 实例，包含 3 个有效 KV
**When** 调用 `fdb_kv_iterator_init` 初始化迭代器，然后循环调用 `fdb_kv_iterate`
**Then** 迭代器产出 3 个 KV
**And** 每个产出的 KV 状态为 `FDB_KV_WRITE` 且 CRC 校验通过

#### Scenario: 重置为默认值

**Given** 一个已初始化的 KVDB 实例，包含用户修改过的 KV
**When** 调用 `fdb_kv_set_default(db)` 重置
**Then** 返回值等于 `FDB_NO_ERR`
**And** 所有 KV 恢复为 `default_kv` 参数中定义的默认值

### Sad Path

#### Scenario: 初始化时分区不存在

**Given** 一个 KVDB 实例，FAL 分区 "nonexistent_part" 不存在
**When** 调用 `fdb_kvdb_init(db, "config", "nonexistent_part", NULL, NULL)`
**Then** 返回值等于 `FDB_PART_NOT_FOUND`
**And** 实例的 `init_ok` 标志为 false

#### Scenario: 扇区大小不是 2 的幂

**Given** 一个 KVDB 实例，通过 control 命令设置 `sec_size = 3000`（非 2 的幂）
**When** 调用 `fdb_kvdb_init`
**Then** 触发 FDB_ASSERT 断言失败

#### Scenario: 数据库总大小不是扇区大小的整数倍

**Given** 一个 KVDB 实例，`sec_size = 4096`，分区总大小 = 10000（非 4096 整数倍）
**When** 调用 `fdb_kvdb_init`
**Then** 返回值等于 `FDB_INIT_FAILED`

#### Scenario: 扇区数不足 2 个

**Given** 一个 KVDB 实例，`sec_size = 4096`，分区总大小 = 4096（仅 1 个扇区）
**When** 调用 `fdb_kvdb_init`
**Then** 返回值等于 `FDB_INIT_FAILED`

#### Scenario: 写入超长 key 名

**Given** 一个已初始化的 KVDB 实例
**When** 调用 `fdb_kv_set` 写入 key 名长度超过 64 字符
**Then** 返回值等于 `FDB_KV_NAME_ERR`

#### Scenario: 写入超大 KV（超过扇区容量）

**Given** 一个已初始化的 KVDB 实例，扇区大小为 4096 字节
**When** 调用 `fdb_kv_set_blob` 写入总长度（头+名+值）超过 `4096 - SECTOR_HDR_DATA_SIZE` 的 KV
**Then** 返回值等于 `FDB_SAVED_FULL`

#### Scenario: 删除不存在的 key

**Given** 一个已初始化的 KVDB 实例，键 "missing_key" 不存在
**When** 调用 `fdb_kv_del(db, "missing_key")`
**Then** 返回值等于 `FDB_KV_NAME_ERR`

#### Scenario: 未初始化时调用 CRUD 操作

**Given** 一个未初始化的 KVDB 实例（`init_ok == false`）
**When** 调用 `fdb_kv_set(db, "key", "value")`
**Then** 返回值等于 `FDB_INIT_FAILED`

#### Scenario: 读取字符串值但实际是二进制数据

**Given** 一个已初始化的 KVDB 实例，键 "bin_data" 存储了包含不可打印字符的二进制数据
**When** 调用 `fdb_kv_get(db, "bin_data")`
**Then** 返回值为 NULL

#### Scenario: 数据库空间耗尽且 GC 无法释放

**Given** 一个已初始化的 KVDB 实例，所有扇区已满且所有 KV 均为有效状态（无可回收垃圾）
**When** 调用 `fdb_kv_set_blob` 写入新 KV
**Then** 返回值等于 `FDB_SAVED_FULL`

### Edge Cases

#### Scenario: 重复初始化（幂等性）

**Given** 一个已初始化的 KVDB 实例（`init_ok == true`）
**When** 再次调用 `fdb_kvdb_init`
**Then** 返回值等于 `FDB_NO_ERR`（不重复执行初始化）

#### Scenario: 掉电恢复 — 写入中断（PRE_WRITE 状态）

**Given** 一个 KVDB 实例，上次运行时写入 KV 头部后、写入值前掉电（KV 状态为 PRE_WRITE）
**When** 重新调用 `fdb_kvdb_init`
**Then** 返回值等于 `FDB_NO_ERR`
**And** 中断的 KV 被标记为 `FDB_KV_ERR_HDR`
**And** 该 KV 不会出现在迭代结果中

#### Scenario: 掉电恢复 — 删除中断（PRE_DELETE 状态）

**Given** 一个 KVDB 实例，上次运行时更新 KV 时，旧 KV 标记为 PRE_DELETE 后、新 KV 写入前掉电
**When** 重新调用 `fdb_kvdb_init`
**Then** 返回值等于 `FDB_NO_ERR`
**And** 旧 KV 的值被恢复（通过 `move_kv` 搬运恢复）

#### Scenario: 掉电恢复 — GC 中断（dirty=GC 状态）

**Given** 一个 KVDB 实例，上次运行时 GC 搬运过程中掉电（扇区 dirty=GC）
**When** 重新调用 `fdb_kvdb_init`
**Then** 返回值等于 `FDB_NO_ERR`
**And** GC 被自动重新执行完成

#### Scenario: 所有扇区头部损坏

**Given** 一个 KVDB 实例，所有扇区的 magic word 均不正确
**When** 调用 `fdb_kvdb_init`（`not_formatable == false`）
**Then** 返回值等于 `FDB_NO_ERR`
**And` 所有 KV 被重置为默认值（`fdb_kv_set_default` 被调用）

#### Scenario: not_formatable 模式下扇区损坏

**Given** 一个 KVDB 实例，部分扇区头部损坏，`not_formatable == true`
**When** 调用 `fdb_kvdb_init`
**Then** 返回值等于 `FDB_READ_ERR`（不自动修复）

#### Scenario: 版本自动升级 — 新增默认 KV

**Given** 一个已初始化的 KVDB 实例，`ver_num` 为 1，存在 `__ver_num__` KV 值为 1
**When` 固件升级后 `ver_num` 改为 2，默认 KV 集合新增了 "new_param" 键
**Then** 重新初始化后，`__ver_num__` KV 值为 2
**And** 新增的 "new_param" 键被自动创建
**And** 已有的 KV 值不被覆盖

#### Scenario: GC 回收已删除 KV 的空间

**Given** 一个 KVDB 实例，扇区 A 有 3 个 KV，其中 2 个已标记为 DELETED
**When** GC 被触发（空间不足或初始化恢复）
**Then** 扇区 A 中的 1 个有效 KV 被搬运到其他扇区
**And** 扇区 A 被擦除并格式化为 EMPTY 状态
**And** 迭代遍历仍能找到那个有效 KV

#### Scenario: fdb_kv_get 不可重入

**Given** 一个已初始化的 KVDB 实例
**When** 在多线程环境下并发调用 `fdb_kv_get`（未注册锁函数）
**Then** 返回的字符串可能被覆盖（静态缓冲区）

---

## Acceptance Criteria

1. `fdb_kvdb_init` 在空分区上初始化后，所有默认 KV 可读且值正确
2. `fdb_kv_set` 后 `fdb_kv_get` 返回写入的值（字符串和 blob 均适用）
3. 更新已有 KV 后，读取返回新值（非旧值）
4. `fdb_kv_del` 后，`fdb_kv_get_blob` 返回 0
5. `fdb_kv_iterate` 产出的 KV 数量等于数据库中 `FDB_KV_WRITE` 状态且 CRC 有效 KV 的数量
6. `fdb_kv_set_default` 后，所有 KV 恢复为默认值
7. 分区不存在时 `fdb_kvdb_init` 返回 `FDB_PART_NOT_FOUND`
8. 扇区大小非 2 的幂时触发断言
9. 扇区数 < 2 时返回 `FDB_INIT_FAILED`
10. key 长度 > 64 时返回 `FDB_KV_NAME_ERR`
11. 未初始化时 CRUD 操作返回 `FDB_INIT_FAILED`
12. 掉电恢复后，PRE_WRITE 状态的 KV 被标记为 ERR_HDR
13. 掉电恢复后，PRE_DELETE 状态的 KV 被恢复
14. 掉电恢复后，dirty=GC 的扇区被重新 GC
15. GC 后已删除 KV 的空间被回收
16. 重复初始化是幂等的（返回 `FDB_NO_ERR`）

---

## Non-Functional Requirements

- **性能**: 单次 KV 写入 < 10ms（NOR Flash W25Q64），单次 KV 读取 < 5ms
- **可靠性**: 掉电后数据一致性通过两阶段写入和状态机恢复保证
- **资源占用**: RAM 占用接近 0（仅 KV 缓存表可选），代码体积 < 5KB（IAR 优化）
- **Flash 寿命**: 通过 GC 隐式实现磨损均衡，避免单个扇区过度擦写
