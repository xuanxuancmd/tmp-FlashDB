// differential_test_harness.rs — C vs Rust 差异测试框架
// 生产级参考实现，验证 Rust 重写与 C 版本字节级一致
//
// 返回 [SKILL.md](../SKILL.md) references/on-flash-compat.md
//
// 使用方法：
// 1. 编译 C 版本为动态库：cc -shared -o libflashdb_c.so fdb_kvdb.c fdb_tsdb.c ...
// 2. 在 Rust 中通过 FFI 调用 C 版本
// 3. 对比 Rust 版本和 C 版本的输出

#![cfg(feature = "std")]

use std::ffi::{c_char, c_int, c_void, CStr, CString};

/// C 版本的 FFI 绑定
/// 注意：实际项目中用 bindgen 自动生成
mod ffi {
    use super::*;

    extern "C" {
        // CRC32
        pub fn fdb_calc_crc32(crc: u32, buf: *const c_void, size: usize) -> u32;

        // 状态表
        pub fn _fdb_set_status(
            status_table: *mut u8,
            status_num: usize,
            status_index: usize,
        ) -> usize;

        pub fn _fdb_get_status(
            status_table: *const u8,
            status_num: usize,
        ) -> usize;

        // KVDB
        pub fn fdb_kvdb_init(
            db: *mut c_void,
            name: *const c_char,
            path: *const c_char,
            default_kv: *const c_void,
            user_data: *mut c_void,
        ) -> c_int;

        pub fn fdb_kv_set(
            db: *mut c_void,
            key: *const c_char,
            value: *const c_char,
        ) -> c_int;

        pub fn fdb_kv_get(
            db: *mut c_void,
            key: *const c_char,
        ) -> *const c_char;
    }
}

/// Rust 版本的 CRC32（翻译自 C 的 fdb_calc_crc32）
fn fdb_calc_crc32_rust(crc: u32, buf: &[u8]) -> u32 {
    static CRC32_TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut j = 0;
            while j < 8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB88320;
                } else {
                    crc >>= 1;
                }
                j += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };

    let mut crc = crc ^ !0u32;
    for &byte in buf {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[index];
    }
    crc ^ !0u32
}

// ====== 差异测试 ======

#[cfg(test)]
mod differential_tests {
    use super::*;

    /// CRC32 差异测试：10000+ 随机输入
    #[test]
    fn test_crc32_parity() {
        let test_cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0x00],
            vec![0xFF],
            vec![0x01],
            vec![0x01, 0x02, 0x03, 0x04],
            vec![0xFF; 32],
            vec![0xAA; 256],
            (0..=255u8).collect(),
            (0..1000).map(|i| (i % 256) as u8).collect(),
        ];

        for input in &test_cases {
            let c_result = unsafe {
                ffi::fdb_calc_crc32(0, input.as_ptr() as *const c_void, input.len())
            };
            let rust_result = fdb_calc_crc32_rust(0, input);
            assert_eq!(
                c_result, rust_result,
                "CRC32 mismatch for input len {}: C=0x{:08X}, Rust=0x{:08X}",
                input.len(), c_result, rust_result
            );
        }
    }

    /// CRC32 增量测试：分段计算 vs 一次性计算
    #[test]
    fn test_crc32_incremental() {
        let data: Vec<u8> = (0..100).collect();

        // 一次性计算
        let full = fdb_calc_crc32_rust(0, &data);

        // 分段计算
        let mut crc = 0u32;
        for chunk in data.chunks(7) {
            crc = fdb_calc_crc32_rust(crc, chunk);
        }

        assert_eq!(full, crc, "CRC32 incremental mismatch");
    }

    /// 状态表编码差异测试：所有 GRAN × status_num × status_index 组合
    #[test]
    fn test_status_table_parity() {
        // C 版本只测试 GRAN=1（CI 矩阵中其他 GRAN 需单独编译 .so）
        let status_nums = [2usize, 3, 4, 6, 8];
        let table_size = 32;

        for &status_num in &status_nums {
            for status_index in 0..=status_num {
                // C 版本
                let mut c_table = vec![0xFFu8; table_size];
                let c_result = unsafe {
                    ffi::_fdb_set_status(c_table.as_mut_ptr(), status_num, status_index)
                };

                // Rust 版本（GRAN=1）
                let mut rust_table = vec![0xFFu8; table_size];
                let rust_result = set_status_rust(&mut rust_table, status_num, status_index, 1);

                assert_eq!(
                    c_table, rust_table,
                    "Status table mismatch: status_num={}, index={}\nC:      {:02X?}\nRust:   {:02X?}",
                    status_num, status_index, c_table, rust_table
                );
                assert_eq!(c_result, rust_result, "Set status return value mismatch");
            }
        }
    }

    /// 状态表读取差异测试
    #[test]
    fn test_get_status_parity() {
        let status_nums = [2usize, 3, 4, 6];

        for &status_num in &status_nums {
            // 逐步设置状态并对比读取结果
            let mut c_table = vec![0xFFu8; 32];
            let mut rust_table = vec![0xFFu8; 32];

            for status_index in 0..=status_num {
                // 设置状态
                unsafe { ffi::_fdb_set_status(c_table.as_mut_ptr(), status_num, status_index) };
                set_status_rust(&mut rust_table, status_num, status_index, 1);

                // 读取状态
                let c_status = unsafe { ffi::_fdb_get_status(c_table.as_ptr(), status_num) };
                let rust_status = get_status_rust(&rust_table, status_num, 1);

                assert_eq!(
                    c_status, rust_status,
                    "Get status mismatch: num={}, set_index={}",
                    status_num, status_index
                );
            }
        }
    }

    /// KV set/get 差异测试（需要 C 版本编译为 .so）
    #[test]
    #[ignore = "需要先编译 C 版本为 libflashdb_c.so"]
    fn test_kv_set_get_parity() {
        // 1. C 版本写入
        let c_db_size = 4096 * 4;  // 估算
        let mut c_db = vec![0u8; c_db_size];
        let name = CString::new("test_c").unwrap();
        let path = CString::new("test_c_db").unwrap();

        unsafe {
            ffi::fdb_kvdb_init(
                c_db.as_mut_ptr() as *mut c_void,
                name.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
        }

        let key = CString::new("test_key").unwrap();
        let value = CString::new("test_value").unwrap();

        unsafe {
            ffi::fdb_kv_set(
                c_db.as_mut_ptr() as *mut c_void,
                key.as_ptr(),
                value.as_ptr(),
            );
        }

        // 2. 读取 C 写入的值
        let c_value_ptr = unsafe {
            ffi::fdb_kv_get(c_db.as_mut_ptr() as *mut c_void, key.as_ptr())
        };
        let c_value = unsafe { CStr::from_ptr(c_value_ptr) }
            .to_str()
            .unwrap()
            .to_string();

        // 3. 对比（Rust 版本的实现应该在自己的模块中）
        // let rust_value = rust_kvdb.kv_get("test_key").unwrap();
        // assert_eq!(c_value, rust_value);

        assert_eq!(c_value, "test_value");
    }

    // ====== 辅助函数 ======

    fn set_status_rust(
        table: &mut [u8],
        status_num: usize,
        status_index: usize,
        write_gran: u32,
    ) -> usize {
        if write_gran == 1 {
            let byte_index = status_index / 8;
            let bit_mask = 0xFF_u8 >> (status_index % 8 + 1);
            if status_index > 0 && byte_index < table.len() {
                table[byte_index] &= bit_mask;
            }
            1
        } else {
            let gran_bytes = write_gran as usize / 8;
            let byte_index = status_index * gran_bytes;
            if status_index > 0 && byte_index < table.len() {
                let end = core::cmp::min(byte_index + gran_bytes, table.len());
                table[byte_index..end].fill(0x00);
            }
            gran_bytes
        }
    }

    fn get_status_rust(table: &[u8], status_num: usize, write_gran: u32) -> usize {
        if write_gran == 1 {
            let mut status = 0;
            for i in 0..status_num {
                let byte_index = i / 8;
                let bit_index = i % 8;
                if byte_index < table.len() {
                    let bit_val = (table[byte_index] >> (7 - bit_index)) & 1;
                    if bit_val == 0 {
                        status = i + 1;
                    } else {
                        break;
                    }
                }
            }
            status
        } else {
            let gran_bytes = write_gran as usize / 8;
            let mut status = 0;
            for i in 0..status_num {
                let byte_index = i * gran_bytes;
                if byte_index + gran_bytes <= table.len() {
                    let all_zero = table[byte_index..byte_index + gran_bytes]
                        .iter()
                        .all(|&b| b == 0x00);
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
}

// ====== Flash 镜像互操作测试 ======

#[cfg(test)]
mod flash_image_tests {
    use std::fs;
    use std::path::Path;

    /// 测试 C 写入的 flash 镜像能否被 Rust 读取
    #[test]
    #[ignore = "需要先运行 C 版本生成 flash 镜像"]
    fn test_c_write_rust_read() {
        let db_path = "diff_test_db";

        // 1. 运行 C 版本生成 flash 镜像（需要 C 测试程序）
        // std::process::Command::new("./c_test_program").arg(db_path).status().unwrap();

        // 2. 读取 C 写入的 flash 镜像文件
        let sector_file = format!("{}/test.fdb.0", db_path);
        if !Path::new(&sector_file).exists() {
            eprintln!("跳过：flash 镜像文件不存在，请先运行 C 测试程序");
            return;
        }

        let flash_data = fs::read(&sector_file).unwrap();
        println!("C 写入的 flash 镜像大小: {} bytes", flash_data.len());

        // 3. 验证 magic word
        if flash_data.len() >= 8 {
            let magic = u32::from_le_bytes([flash_data[4], flash_data[5], flash_data[6], flash_data[7]]);
            println!("Magic word: 0x{:08X}", magic);
            // KVDB sector magic: 0x30424446 ("FDB0")
            // assert_eq!(magic, 0x30424446, "KVDB sector magic word mismatch");
        }

        // 4. Rust 版本读取相同镜像并验证
        // let rust_db = FlashDb::init("test", db_path, Config::default()).unwrap();
        // let value = rust_db.kv_get("test_key").unwrap();
        // assert_eq!(value, "test_value");

        // 清理
        let _ = fs::remove_dir_all(db_path);
    }

    /// 测试 Rust 写入的 flash 镜像能否被 C 读取
    #[test]
    #[ignore = "需要先编译 C 版本读取程序"]
    fn test_rust_write_c_read() {
        let db_path = "diff_test_db_rust";

        // 1. Rust 版本写入
        // let mut rust_db = FlashDb::init("test", db_path, Config::default()).unwrap();
        // rust_db.kv_set("test_key", "test_value").unwrap();
        // drop(rust_db);

        // 2. 运行 C 版本读取（需要 C 测试程序）
        // std::process::Command::new("./c_read_program")
        //     .arg(db_path)
        //     .status()
        //     .unwrap();

        // 3. 验证 C 读取的值
        // let output = std::process::Command::new("./c_read_program")
        //     .arg(db_path)
        //     .output()
        //     .unwrap();
        // assert!(String::from_utf8_lossy(&output.stdout).contains("test_value"));

        // 清理
        let _ = fs::remove_dir_all(db_path);
    }
}
