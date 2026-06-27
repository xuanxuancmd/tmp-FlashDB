// status_table_encoding.rs — 状态表位编码（6 种 WRITE_GRAN 变体）
// 生产级参考实现，替代 C 的 _fdb_set_status / _fdb_get_status
//
// 返回 [SKILL.md](../SKILL.md) references/feature-flags.md

#![cfg_attr(not(feature = "std"), no_std)]

/// 状态表编码器 —— 利用 NOR flash 的 1→0 单向写入特性实现状态机
///
/// C 版本用 `#if (FDB_WRITE_GRAN == 1)` 选择不同编码策略，
/// Rust 版本用 const generic 在编译时单态化。
///
/// 状态编码原理：
/// - FDB_WRITE_GRAN=1（bit 级）：每个状态占 1 bit，逐位翻转
///   状态 0: 0xFF → 状态 1: 0x7F → 状态 2: 0x3F → 状态 3: 0x1F
/// - FDB_WRITE_GRAN=8（byte 级）：每个状态占 1 byte
///   状态 0: 0xFF → 状态 1: 0x00 → 状态 2: 0x00 0x00
/// - FDB_WRITE_GRAN=32/64/128/256：每个状态占多字节
pub struct StatusTable<const WRITE_GRAN: u32>;

/// Flash 字节常量
pub const FDB_BYTE_ERASED: u8 = 0xFF;
pub const FDB_BYTE_WRITTEN: u8 = 0x00;

impl<const WRITE_GRAN: u32> StatusTable<WRITE_GRAN> {
    /// 编译时验证 GRAN 合法值
    const _: () = {
        assert!(matches!(WRITE_GRAN, 1 | 8 | 32 | 64 | 128 | 256));
    };

    /// 计算状态表字节大小
    ///
    /// C: `#define FDB_STATUS_TABLE_SIZE(status_number) ((status_number * FDB_WRITE_GRAN + 7)/8)`
    /// 或 `(((status_number - 1) * FDB_WRITE_GRAN + 7)/8)`
    pub const fn table_size(status_num: usize) -> usize {
        if WRITE_GRAN == 1 {
            // bit 级：每个状态占 1 bit
            (status_num * 1 + 7) / 8
        } else {
            // byte/word 级：每个状态占 (GRAN/8) 字节
            ((status_num - 1) * (WRITE_GRAN as usize / 8) + 7) / 8
        }
    }

    /// 设置状态（写入状态表）
    ///
    /// C: `size_t _fdb_set_status(uint8_t status_table[], size_t status_num, size_t status_index)`
    ///
    /// 返回写入的字节大小
    pub fn set_status(status_table: &mut [u8], status_num: usize, status_index: usize) -> usize {
        if WRITE_GRAN == 1 {
            // bit 级编码：每个状态翻转 1 bit
            let byte_index = status_index / 8;
            let bit_mask = 0xFF_u8 >> (status_index % 8 + 1);

            if status_index > 0 {
                status_table[byte_index] &= bit_mask;
            }
            1
        } else {
            // byte/word 级编码：每个状态写一个完整单元为 0x00
            let gran_bytes = WRITE_GRAN as usize / 8;
            let byte_index = status_index * gran_bytes;

            if status_index > 0 && byte_index < status_table.len() {
                let end = core::cmp::min(byte_index + gran_bytes, status_table.len());
                status_table[byte_index..end].fill(FDB_BYTE_WRITTEN);
            }
            gran_bytes
        }
    }

    /// 读取当前状态
    ///
    /// C: `size_t _fdb_get_status(uint8_t status_table[], size_t status_num)`
    pub fn get_status(status_table: &[u8], status_num: usize) -> usize {
        if WRITE_GRAN == 1 {
            // bit 级：计算连续 0 bit 的数量
            let mut status = 0;
            for i in 0..status_num {
                let byte_index = i / 8;
                let bit_index = i % 8;
                if byte_index < status_table.len() {
                    let bit_val = (status_table[byte_index] >> (7 - bit_index)) & 1;
                    if bit_val == 0 {
                        status = i + 1;
                    } else {
                        break;
                    }
                }
            }
            status
        } else {
            // byte/word 级：计算连续 0x00 单元的数量
            let gran_bytes = WRITE_GRAN as usize / 8;
            let mut status = 0;
            for i in 0..status_num {
                let byte_index = i * gran_bytes;
                if byte_index + gran_bytes <= status_table.len() {
                    let all_zero = status_table[byte_index..byte_index + gran_bytes]
                        .iter()
                        .all(|&b| b == FDB_BYTE_WRITTEN);
                    if all_zero {
                        status = i + 1;
                    } else {
                        break;
                    }
                }
            }
            status
        }
    }

    /// 对齐计算
    ///
    /// C: `#define FDB_WG_ALIGN(size) (FDB_ALIGN(size, ((FDB_WRITE_GRAN + 7)/8)))`
    pub const fn wg_align(size: usize) -> usize {
        let align = (WRITE_GRAN as usize + 7) / 8;
        (size + align - 1) / align * align
    }

    /// 向下对齐
    ///
    /// C: `#define FDB_WG_ALIGN_DOWN(size) (FDB_ALIGN_DOWN(size, (FDB_WRITE_GRAN + 7)/8))`
    pub const fn wg_align_down(size: usize) -> usize {
        let align = (WRITE_GRAN as usize + 7) / 8;
        size / align * align
    }
}

// ====== 编译时大小验证 ======

// GRAN=1: 4 个状态需要 4 bit = 1 byte
const _: () = assert!(StatusTable::<1>::table_size(4) >= 1);
// GRAN=8: 4 个状态需要 3 byte（(4-1)*1 = 3）
const _: () = assert!(StatusTable::<8>::table_size(4) >= 3);
// GRAN=32: 4 个状态需要 12 byte（(4-1)*4 = 12）
const _: () = assert!(StatusTable::<32>::table_size(4) >= 12);

#[cfg(test)]
mod tests {
    use super::*;

    // ====== GRAN=1 (NOR flash) 测试 ======

    #[test]
    fn test_gran1_set_get_status() {
        let mut table = [FDB_BYTE_ERASED; 4];

        // 初始状态 = 0
        assert_eq!(StatusTable::<1>::get_status(&table, 4), 0);

        // 设置状态 1
        StatusTable::<1>::set_status(&mut table, 4, 1);
        assert_eq!(StatusTable::<1>::get_status(&table, 4), 1);

        // 设置状态 2
        StatusTable::<1>::set_status(&mut table, 4, 2);
        assert_eq!(StatusTable::<1>::get_status(&table, 4), 2);

        // 设置状态 3
        StatusTable::<1>::set_status(&mut table, 4, 3);
        assert_eq!(StatusTable::<1>::get_status(&table, 4), 3);
    }

    #[test]
    fn test_gran1_nor_flash_semantics() {
        // GRAN=1 时，状态编码利用 NOR flash 1→0 特性
        // 状态 0: 0xFF (11111111)
        // 状态 1: 0x7F (01111111) - 翻转 bit 7
        // 状态 2: 0x3F (00111111) - 翻转 bit 6
        // 状态 3: 0x1F (00011111) - 翻转 bit 5

        let mut table = [FDB_BYTE_ERASED; 1];

        StatusTable::<1>::set_status(&mut table, 4, 1);
        assert_eq!(table[0], 0x7F);

        StatusTable::<1>::set_status(&mut table, 4, 2);
        assert_eq!(table[0], 0x3F);

        StatusTable::<1>::set_status(&mut table, 4, 3);
        assert_eq!(table[0], 0x1F);
    }

    // ====== GRAN=8 (STM32F2/F4) 测试 ======

    #[test]
    fn test_gran8_set_get_status() {
        let mut table = [FDB_BYTE_ERASED; 8];

        assert_eq!(StatusTable::<8>::get_status(&table, 4), 0);

        StatusTable::<8>::set_status(&mut table, 4, 1);
        assert_eq!(StatusTable::<8>::get_status(&table, 4), 1);

        StatusTable::<8>::set_status(&mut table, 4, 2);
        assert_eq!(StatusTable::<8>::get_status(&table, 4), 2);

        StatusTable::<8>::set_status(&mut table, 4, 3);
        assert_eq!(StatusTable::<8>::get_status(&table, 4), 3);
    }

    #[test]
    fn test_gran8_byte_encoding() {
        // GRAN=8: 每个状态写 1 byte 为 0x00
        let mut table = [FDB_BYTE_ERASED; 4];

        StatusTable::<8>::set_status(&mut table, 4, 1);
        assert_eq!(table[0], FDB_BYTE_WRITTEN);  // 0x00

        StatusTable::<8>::set_status(&mut table, 4, 2);
        assert_eq!(table[1], FDB_BYTE_WRITTEN);

        StatusTable::<8>::set_status(&mut table, 4, 3);
        assert_eq!(table[2], FDB_BYTE_WRITTEN);
    }

    // ====== GRAN=32 (STM32F1) 测试 ======

    #[test]
    fn test_gran32_set_get_status() {
        let mut table = [FDB_BYTE_ERASED; 16];

        assert_eq!(StatusTable::<32>::get_status(&table, 4), 0);

        StatusTable::<32>::set_status(&mut table, 4, 1);
        assert_eq!(StatusTable::<32>::get_status(&table, 4), 1);

        StatusTable::<32>::set_status(&mut table, 4, 2);
        assert_eq!(StatusTable::<32>::get_status(&table, 4), 2);
    }

    // ====== 对齐测试 ======

    #[test]
    fn test_wg_align_gran1() {
        // GRAN=1: align = (1+7)/8 = 1
        assert_eq!(StatusTable::<1>::wg_align(1), 1);
        assert_eq!(StatusTable::<1>::wg_align(7), 7);
        assert_eq!(StatusTable::<1>::wg_align(8), 8);
    }

    #[test]
    fn test_wg_align_gran8() {
        // GRAN=8: align = (8+7)/8 = 1
        assert_eq!(StatusTable::<8>::wg_align(1), 1);
        assert_eq!(StatusTable::<8>::wg_align(7), 7);
    }

    #[test]
    fn test_wg_align_gran32() {
        // GRAN=32: align = (32+7)/8 = 4
        assert_eq!(StatusTable::<32>::wg_align(1), 4);
        assert_eq!(StatusTable::<32>::wg_align(4), 4);
        assert_eq!(StatusTable::<32>::wg_align(5), 8);
        assert_eq!(StatusTable::<32>::wg_align(8), 8);
    }

    #[test]
    fn test_wg_align_gran64() {
        // GRAN=64: align = (64+7)/8 = 8
        assert_eq!(StatusTable::<64>::wg_align(1), 8);
        assert_eq!(StatusTable::<64>::wg_align(8), 8);
        assert_eq!(StatusTable::<64>::wg_align(9), 16);
    }

    #[test]
    fn test_wg_align_down() {
        assert_eq!(StatusTable::<32>::wg_align_down(7), 4);
        assert_eq!(StatusTable::<32>::wg_align_down(8), 8);
        assert_eq!(StatusTable::<32>::wg_align_down(9), 8);
    }

    // ====== 表大小计算测试 ======

    #[test]
    fn test_table_size() {
        // GRAN=1: 6 状态需要 6 bit = 1 byte
        assert_eq!(StatusTable::<1>::table_size(6), 1);

        // GRAN=8: 6 状态需要 (6-1)*1 = 5 byte
        assert_eq!(StatusTable::<8>::table_size(6), 5);

        // GRAN=32: 6 状态需要 (6-1)*4 = 20 byte
        assert_eq!(StatusTable::<32>::table_size(6), 20);
    }
}
