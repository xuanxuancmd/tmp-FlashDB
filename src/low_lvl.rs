// c: fdb_low_lvl.h + fdb_utils.c — Low-level API (constants, alignment,
// status table, flash I/O wrappers, CRC32, blob API)
//
// This module is the 1:1 Rust translation of fdb_low_lvl.h (macros) and
// fdb_utils.c (implementation). The C `_fdb_flash_*` functions dispatched on
// `db->file_mode` to either FAL or file backends; in Rust the flash backend is
// supplied via the `FlashDevice` trait (see flash_trait.rs and skill
// references/fal-to-trait.md), so the dispatch collapses to a direct trait call.

#![allow(dead_code)]

use crate::def::{FdbBlob, FdbErr, FDB_BYTE_ERASED, FDB_BYTE_WRITTEN, FDB_DATA_UNUSED, FDB_FAILED_ADDR, FDB_WRITE_GRAN};
use crate::flash_trait::FlashDevice;

// ===== Alignment helpers (c: fdb_low_lvl.h:29-41) =====

/// c: fdb_low_lvl.h:32 — FDB_ALIGN(size, align): round `size` up to a multiple of `align`.
/// FDB_ALIGN(13, 4) == 16.
pub(crate) const fn align_up(size: u32, align: u32) -> u32 {
    (size + align - 1) - ((size + align - 1) % align)
}

/// c: fdb_low_lvl.h:34 — FDB_WG_ALIGN(size): align `size` up by write granularity.
pub(crate) const fn wg_align(size: u32) -> u32 {
    align_up(size, (FDB_WRITE_GRAN + 7) / 8)
}

/// c: fdb_low_lvl.h:39 — FDB_ALIGN_DOWN(size, align): round `size` down to a multiple of `align`.
/// FDB_ALIGN_DOWN(13, 4) == 12.
pub(crate) const fn align_down(size: u32, align: u32) -> u32 {
    (size / align) * align
}

/// c: fdb_low_lvl.h:41 — FDB_WG_ALIGN_DOWN(size): align `size` down by write granularity.
pub(crate) const fn wg_align_down(size: u32) -> u32 {
    align_down(size, (FDB_WRITE_GRAN + 7) / 8)
}

// ===== Status table size (c: fdb_low_lvl.h:18-22) =====

/// c: fdb_low_lvl.h:18-22 — FDB_STATUS_TABLE_SIZE(status_number) macro.
///
/// For GRAN==1 the table holds `status_num` bits packed into bytes.
/// For GRAN>1 each status occupies GRAN/8 bytes (one byte written per step).
pub(crate) const fn status_table_size(status_num: u32) -> u32 {
    if FDB_WRITE_GRAN == 1 {
        (status_num * FDB_WRITE_GRAN + 7) / 8
    } else {
        ((status_num - 1) * FDB_WRITE_GRAN + 7) / 8
    }
}

// ===== CRC32 (c: fdb_utils.c:21-89) =====

/// c: fdb_utils.c:21-66 — CRC32 lookup table (256 entries, polynomial 0xEDB88320).
///
/// Copied verbatim from fdb_utils.c; must remain byte-for-byte identical so that
/// on-flash CRC values stay compatible with the C version.
static CRC32_TABLE: [u32; 256] = [
    0x00000000, 0x77073096, 0xee0e612c, 0x990951ba, 0x076dc419, 0x706af48f,
    0xe963a535, 0x9e6495a3, 0x0edb8832, 0x79dcb8a4, 0xe0d5e91e, 0x97d2d988,
    0x09b64c2b, 0x7eb17cbd, 0xe7b82d07, 0x90bf1d91, 0x1db71064, 0x6ab020f2,
    0xf3b97148, 0x84be41de, 0x1adad47d, 0x6ddde4eb, 0xf4d4b551, 0x83d385c7,
    0x136c9856, 0x646ba8c0, 0xfd62f97a, 0x8a65c9ec, 0x14015c4f, 0x63066cd9,
    0xfa0f3d63, 0x8d080df5, 0x3b6e20c8, 0x4c69105e, 0xd56041e4, 0xa2677172,
    0x3c03e4d1, 0x4b04d447, 0xd20d85fd, 0xa50ab56b, 0x35b5a8fa, 0x42b2986c,
    0xdbbbc9d6, 0xacbcf940, 0x32d86ce3, 0x45df5c75, 0xdcd60dcf, 0xabd13d59,
    0x26d930ac, 0x51de003a, 0xc8d75180, 0xbfd06116, 0x21b4f4b5, 0x56b3c423,
    0xcfba9599, 0xb8bda50f, 0x2802b89e, 0x5f058808, 0xc60cd9b2, 0xb10be924,
    0x2f6f7c87, 0x58684c11, 0xc1611dab, 0xb6662d3d, 0x76dc4190, 0x01db7106,
    0x98d220bc, 0xefd5102a, 0x71b18589, 0x06b6b51f, 0x9fbfe4a5, 0xe8b8d433,
    0x7807c9a2, 0x0f00f934, 0x9609a88e, 0xe10e9818, 0x7f6a0dbb, 0x086d3d2d,
    0x91646c97, 0xe6635c01, 0x6b6b51f4, 0x1c6c6162, 0x856530d8, 0xf262004e,
    0x6c0695ed, 0x1b01a57b, 0x8208f4c1, 0xf50fc457, 0x65b0d9c6, 0x12b7e950,
    0x8bbeb8ea, 0xfcb9887c, 0x62dd1ddf, 0x15da2d49, 0x8cd37cf3, 0xfbd44c65,
    0x4db26158, 0x3ab551ce, 0xa3bc0074, 0xd4bb30e2, 0x4adfa541, 0x3dd895d7,
    0xa4d1c46d, 0xd3d6f4fb, 0x4369e96a, 0x346ed9fc, 0xad678846, 0xda60b8d0,
    0x44042d73, 0x33031de5, 0xaa0a4c5f, 0xdd0d7cc9, 0x5005713c, 0x270241aa,
    0xbe0b1010, 0xc90c2086, 0x5768b525, 0x206f85b3, 0xb966d409, 0xce61e49f,
    0x5edef90e, 0x29d9c998, 0xb0d09822, 0xc7d7a8b4, 0x59b33d17, 0x2eb40d81,
    0xb7bd5c3b, 0xc0ba6cad, 0xedb88320, 0x9abfb3b6, 0x03b6e20c, 0x74b1d29a,
    0xead54739, 0x9dd277af, 0x04db2615, 0x73dc1683, 0xe3630b12, 0x94643b84,
    0x0d6d6a3e, 0x7a6a5aa8, 0xe40ecf0b, 0x9309ff9d, 0x0a00ae27, 0x7d079eb1,
    0xf00f9344, 0x8708a3d2, 0x1e01f268, 0x6906c2fe, 0xf762575d, 0x806567cb,
    0x196c3671, 0x6e6b06e7, 0xfed41b76, 0x89d32be0, 0x10da7a5a, 0x67dd4acc,
    0xf9b9df6f, 0x8ebeeff9, 0x17b7be43, 0x60b08ed5, 0xd6d6a3e8, 0xa1d1937e,
    0x38d8c2c4, 0x4fdff252, 0xd1bb67f1, 0xa6bc5767, 0x3fb506dd, 0x48b2364b,
    0xd80d2bda, 0xaf0a1b4c, 0x36034af6, 0x41047a60, 0xdf60efc3, 0xa867df55,
    0x316e8eef, 0x4669be79, 0xcb61b38c, 0xbc66831a, 0x256fd2a0, 0x5268e236,
    0xcc0c7795, 0xbb0b4703, 0x220216b9, 0x5505262f, 0xc5ba3bbe, 0xb2bd0b28,
    0x2bb45a92, 0x5cb36a04, 0xc2d7ffa7, 0xb5d0cf31, 0x2cd99e8b, 0x5bdeae1d,
    0x9b64c2b0, 0xec63f226, 0x756aa39c, 0x026d930a, 0x9c0906a9, 0xeb0e363f,
    0x72076785, 0x05005713, 0x95bf4a82, 0xe2b87a14, 0x7bb12bae, 0x0cb61b38,
    0x92d28e9b, 0xe5d5be0d, 0x7cdcefb7, 0x0bdbdf21, 0x86d3d2d4, 0xf1d4e242,
    0x68ddb3f8, 0x1fda836e, 0x81be16cd, 0xf6b9265b, 0x6fb077e1, 0x18b74777,
    0x88085ae6, 0xff0f6a70, 0x66063bca, 0x11010b5c, 0x8f659eff, 0xf862ae69,
    0x616bffd3, 0x166ccf45, 0xa00ae278, 0xd70dd2ee, 0x4e048354, 0x3903b3c2,
    0xa7672661, 0xd06016f7, 0x4969474d, 0x3e6e77db, 0xaed16a4a, 0xd9d65adc,
    0x40df0b66, 0x37d83bf0, 0xa9bcae53, 0xdebb9ec5, 0x47b2cf7f, 0x30b5ffe9,
    0xbdbdf21c, 0xcabac28a, 0x53b39330, 0x24b4a3a6, 0xbad03605, 0xcdd70693,
    0x54de5729, 0x23d967bf, 0xb3667a2e, 0xc4614ab8, 0x5d681b02, 0x2a6f2b94,
    0xb40bbe37, 0xc30c8ea1, 0x5a05df1b, 0x2d02ef8d,
];

/// c: fdb_utils.c:77-89 — Calculate the CRC32 value of a memory buffer.
///
/// `crc` is the accumulated CRC32 value and must be 0 on the first call.
/// This is the standard CRC-32/ISO-HDLC algorithm (init 0xFFFFFFFF, final XOR
/// 0xFFFFFFFF, polynomial 0xEDB88320), matching the C implementation byte-for-byte.
pub fn calc_crc32(crc: u32, buf: &[u8]) -> u32 {
    let mut crc = crc ^ !0u32;
    for &byte in buf {
        // c: crc = crc32_table[(crc ^ *p++) & 0xFF] ^ (crc >> 8);
        crc = CRC32_TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ !0u32
}

// ===== Status table set/get (c: fdb_utils.c:91-145) =====

/// c: fdb_utils.c:91-124 — _fdb_set_status
///
/// Encode `status_index` into `status_table` (which has `status_num` slots).
/// Returns the byte index that was modified, or `usize::MAX` when the first
/// status (index 0, all-erased) is selected and thus nothing needs writing.
pub fn set_status(status_table: &mut [u8], status_num: usize, status_index: usize) -> usize {
    // c: memset(status_table, FDB_BYTE_ERASED, FDB_STATUS_TABLE_SIZE(status_num))
    let table_size = status_table_size(status_num as u32) as usize;
    status_table[..table_size].fill(FDB_BYTE_ERASED);

    let mut byte_index = usize::MAX;
    if status_index > 0 {
        // FDB_BYTE_ERASED == 0xFF branch (the only supported erased value)
        if FDB_WRITE_GRAN == 1 {
            // c: byte_index = (status_index - 1) / 8; status_table[byte_index] &= (0x00ff >> (status_index % 8));
            byte_index = (status_index - 1) / 8;
            status_table[byte_index] &= 0x00ff_u8 >> (status_index % 8);
        } else {
            // c: byte_index = (status_index - 1) * (FDB_WRITE_GRAN / 8); status_table[byte_index] = FDB_BYTE_WRITTEN;
            byte_index = (status_index - 1) * (FDB_WRITE_GRAN as usize / 8);
            status_table[byte_index] = FDB_BYTE_WRITTEN;
        }
    }
    byte_index
}

/// c: fdb_utils.c:126-145 — _fdb_get_status
///
/// Decode the current status index from `status_table` (which has `status_num`
/// slots). Scans from the highest index down to 0, returning the first slot
/// whose bit/byte indicates "written".
pub fn get_status(status_table: &[u8], status_num: usize) -> usize {
    if status_num == 0 {
        return 0;
    }
    // c: size_t i = 0, status_num_bak = --status_num;
    let mut i = 0usize;
    let status_num_bak = status_num - 1;
    // C: while (status_num--) iterates with the post-decremented index taking
    // values status_num-2 .. 0. The equivalent Rust range is (0..=status_num-2).rev().
    if status_num >= 2 {
        for idx in (0..=status_num - 2).rev() {
            let matched = if FDB_WRITE_GRAN == 1 {
                // c: (status_table[status_num / 8] & (0x80 >> (status_num % 8))) == 0x00
                (status_table[idx / 8] & (0x80u8 >> (idx % 8))) == 0x00
            } else {
                // c: status_table[status_num * FDB_WRITE_GRAN / 8] == FDB_BYTE_WRITTEN
                status_table[idx * FDB_WRITE_GRAN as usize / 8] == FDB_BYTE_WRITTEN
            };
            if matched {
                break;
            }
            i += 1;
        }
    }
    // c: return status_num_bak - i;
    status_num_bak - i
}

// ===== Flash I/O wrappers (c: fdb_utils.c:257-349) =====

/// c: fdb_utils.c:257-276 — _fdb_flash_read
///
/// In C this dispatched on `db->file_mode` to FAL or file backend. With the
/// `FlashDevice` trait the dispatch collapses to a direct trait `read` call.
pub fn flash_read<F: FlashDevice>(flash: &F, addr: u32, buf: &mut [u8]) -> Result<(), FdbErr> {
    flash.read(addr, buf)
}

/// c: fdb_utils.c:278-297 — _fdb_flash_erase
pub fn flash_erase<F: FlashDevice>(flash: &mut F, addr: u32, size: u32) -> Result<(), FdbErr> {
    flash.erase(addr, size)
}

/// c: fdb_utils.c:299-320 — _fdb_flash_write
///
/// The C `sync` parameter controlled file-mode flushing; the `FlashDevice`
/// trait write is synchronous, so `sync` is dropped (per Plan T5 must-not-do).
pub fn flash_write<F: FlashDevice>(flash: &mut F, addr: u32, buf: &[u8]) -> Result<(), FdbErr> {
    flash.write(addr, buf)
}

/// c: fdb_utils.c:322-349 — _fdb_flash_write_align
///
/// Writes `buf` to flash, padding the tail up to the write-granularity boundary
/// with `FDB_BYTE_ERASED` (0xFF). For GRAN==1 and GRAN==8 the granularity is a
/// single byte so no padding is ever added; padding only occurs for GRAN>=32.
pub fn flash_write_align<F: FlashDevice>(flash: &mut F, addr: u32, buf: &[u8]) -> Result<(), FdbErr> {
    let size = buf.len();
    // c: align_data_size = FDB_WRITE_GRAN / 8 (== (FDB_WRITE_GRAN + 7) / 8 for GRAN >= 8,
    // and 1 for GRAN == 1 via the C89 compatibility branch)
    let wg_bytes = ((FDB_WRITE_GRAN + 7) / 8) as usize;
    // c: FDB_WG_ALIGN_DOWN(size)
    let aligned_size = wg_align_down(size as u32) as usize;

    // c: memset(align_data, FDB_BYTE_ERASED, align_data_size) — 32 is the max
    // granularity (GRAN==256 -> 32 bytes); the unused tail stays 0xFF.
    let mut align_data = [FDB_BYTE_ERASED; 32];

    // c: if (FDB_WG_ALIGN_DOWN(size) > 0) write the aligned portion
    if aligned_size > 0 {
        flash_write(flash, addr, &buf[..aligned_size])?;
    }

    // c: align_remain = size - FDB_WG_ALIGN_DOWN(size)
    let align_remain = size - aligned_size;
    if align_remain > 0 {
        // c: memcpy(align_data, buf + aligned_size, align_remain)
        align_data[..align_remain].copy_from_slice(&buf[aligned_size..aligned_size + align_remain]);
        // c: write align_data (align_data_size bytes) at addr + aligned_size
        flash_write(flash, addr + aligned_size as u32, &align_data[..wg_bytes])?;
    }
    Ok(())
}

// ===== write_status / read_status (c: fdb_utils.c:147-180) =====

/// c: fdb_utils.c:147-171 — _fdb_write_status
///
/// Encodes `status_index` into `status_table` then persists only the changed
/// byte(s) to flash at `addr`. If the selected status is index 0 (all-erased)
/// nothing is written, mirroring the C fast path.
pub fn write_status<F: FlashDevice>(
    flash: &mut F,
    addr: u32,
    status_table: &mut [u8],
    status_num: usize,
    status_index: usize,
) -> Result<(), FdbErr> {
    // c: FDB_ASSERT(status_index < status_num); FDB_ASSERT(status_table);
    assert!(status_index < status_num, "status_index must be < status_num");

    // c: byte_index = _fdb_set_status(status_table, status_num, status_index);
    let byte_index = set_status(status_table, status_num, status_index);

    // c: if (byte_index == SIZE_MAX) return FDB_NO_ERR;
    if byte_index == usize::MAX {
        return Ok(());
    }

    // c: write the changed byte(s) at addr + byte_index
    let write_len = if FDB_WRITE_GRAN == 1 {
        1
    } else {
        FDB_WRITE_GRAN as usize / 8
    };
    flash_write(
        flash,
        addr + byte_index as u32,
        &status_table[byte_index..byte_index + write_len],
    )
}

/// c: fdb_utils.c:173-180 — _fdb_read_status
///
/// Reads the status table from flash into `status_table` then decodes the index.
/// The C version ignores the `_fdb_flash_read` return value; this translation
/// matches that behaviour (a read failure leaves the buffer contents undefined
/// and decoding proceeds, just as in C).
pub fn read_status<F: FlashDevice>(flash: &F, addr: u32, status_table: &mut [u8], total_num: usize) -> usize {
    // c: FDB_ASSERT(status_table);
    let table_size = status_table_size(total_num as u32) as usize;
    // c: _fdb_flash_read(db, addr, status_table, FDB_STATUS_TABLE_SIZE(total_num));
    let _ = flash_read(flash, addr, &mut status_table[..table_size]);
    // c: return _fdb_get_status(status_table, total_num);
    get_status(status_table, total_num)
}

// ===== continue_ff_addr (c: fdb_utils.c:185-210) =====

/// c: fdb_utils.c:185-210 — _fdb_continue_ff_addr
///
/// Find the address of the first run of contiguous 0xFF bytes that extends to
/// `end`, scanning from `start`. Returns `wg_align(addr)` when the scan ends on
/// an erased byte, otherwise `end`.
pub fn continue_ff_addr<F: FlashDevice>(flash: &F, start: u32, end: u32) -> u32 {
    // c: uint8_t buf[32], last_data = FDB_BYTE_WRITTEN;
    let mut buf = [0u8; 32];
    let mut last_data = FDB_BYTE_WRITTEN;
    let mut addr = start;
    let mut cur = start;

    // c: for (; start < end; start += sizeof(buf))
    while cur < end {
        // c: read_size = (start + sizeof(buf) < end) ? sizeof(buf) : (end - start);
        let read_size = if cur + 32 < end { 32 } else { (end - cur) as usize };
        // c: _fdb_flash_read(db, start, buf, read_size); (C ignores the return)
        let _ = flash_read(flash, cur, &mut buf[..read_size]);
        for i in 0..read_size {
            // c: if (last_data != FDB_BYTE_ERASED && buf[i] == FDB_BYTE_ERASED) addr = start + i;
            if last_data != FDB_BYTE_ERASED && buf[i] == FDB_BYTE_ERASED {
                addr = cur + i as u32;
            }
            last_data = buf[i];
        }
        cur += 32;
    }

    // c: if (last_data == FDB_BYTE_ERASED) return FDB_WG_ALIGN(addr); else return end;
    if last_data == FDB_BYTE_ERASED {
        wg_align(addr)
    } else {
        end
    }
}

// ===== Blob API (c: fdb_utils.c:221-249) =====

/// c: fdb_utils.c:221-227 — fdb_blob_make
///
/// Initialise a blob backed by `buf`. The `saved.*` fields are reset to their
/// unused defaults; the caller fills `saved_addr`/`saved_len` (typically from a
/// KV/TSL header) before invoking `blob_read`.
pub fn blob_make<'a>(buf: &'a mut [u8]) -> FdbBlob<'a> {
    // c: blob->buf = value_buf; blob->size = buf_len;
    let size = buf.len();
    FdbBlob {
        buf,
        size,
        saved_meta_addr: FDB_DATA_UNUSED,
        saved_addr: FDB_FAILED_ADDR,
        saved_len: 0,
    }
}

/// c: fdb_utils.c:237-249 — fdb_blob_read
///
/// Read up to `min(buf.size, saved.len)` bytes from `saved.addr` into the blob
/// buffer. Returns the number of bytes read, or 0 on flash read failure.
pub fn blob_read<F: FlashDevice>(flash: &F, blob: &mut FdbBlob) -> usize {
    // c: size_t read_len = blob->size;
    let mut read_len = blob.size;
    // c: if (read_len > blob->saved.len) read_len = blob->saved.len;
    if read_len > blob.saved_len {
        read_len = blob.saved_len;
    }
    // c: if (_fdb_flash_read(db, blob->saved.addr, blob->buf, read_len) != FDB_NO_ERR) read_len = 0;
    let saved_addr = blob.saved_addr;
    if flash_read(flash, saved_addr, &mut blob.buf[..read_len]).is_err() {
        read_len = 0;
    }
    read_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_flash::MockFlash;

    // ---- alignment helpers ----

    #[test]
    fn test_align_up() {
        // c: FDB_ALIGN(13, 4) == 16
        assert_eq!(align_up(13, 4), 16);
        // already aligned stays the same
        assert_eq!(align_up(16, 4), 16);
        // align to 1 is a no-op
        assert_eq!(align_up(13, 1), 13);
        // align to 8
        assert_eq!(align_up(13, 8), 16);
        assert_eq!(align_up(0, 8), 0);
    }

    #[test]
    fn test_align_down() {
        // c: FDB_ALIGN_DOWN(13, 4) == 12
        assert_eq!(align_down(13, 4), 12);
        assert_eq!(align_down(16, 4), 16);
        assert_eq!(align_down(0, 4), 0);
    }

    #[test]
    fn test_wg_align_gran1() {
        // With the default GRAN==1, the write-granularity is 1 byte, so wg_align
        // and wg_align_down are identity functions.
        assert_eq!(wg_align(13), 13);
        assert_eq!(wg_align_down(15), 15);
        assert_eq!(wg_align(0), 0);
    }

    #[test]
    fn test_status_table_size_gran1() {
        // c: FDB_STATUS_TABLE_SIZE(4) for GRAN==1 == (4*1+7)/8 == 1
        assert_eq!(status_table_size(4), 1);
        // c: FDB_STATUS_TABLE_SIZE(6) for GRAN==1 == (6+7)/8 == 1
        assert_eq!(status_table_size(6), 1);
        // 9 bits -> 2 bytes
        assert_eq!(status_table_size(9), 2);
    }

    // ---- set_status / get_status (GRAN==1) ----

    #[test]
    fn test_set_status_gran1() {
        // status_num=4 (sector store status): Unused=0, Empty=1, Using=2, Full=3
        let mut table = [0u8; 2];

        // index 0 -> all erased, byte_index == SIZE_MAX
        let bi = set_status(&mut table, 4, 0);
        assert_eq!(bi, usize::MAX, "index 0 leaves the table all-erased");
        assert_eq!(table[0], 0xFF);

        // index 1 -> 0xFF & (0x00ff>>1) = 0x7F
        set_status(&mut table, 4, 1);
        assert_eq!(table[0], 0x7F, "index 1 encodes as 0x7F (matches C table)");

        // index 2 -> 0xFF & (0x00ff>>2) = 0x3F
        set_status(&mut table, 4, 2);
        assert_eq!(table[0], 0x3F, "index 2 encodes as 0x3F");

        // index 3 -> 0xFF & (0x00ff>>3) = 0x1F
        set_status(&mut table, 4, 3);
        assert_eq!(table[0], 0x1F, "index 3 encodes as 0x1F");
    }

    #[test]
    fn test_get_status_gran1() {
        // Round-trip every index through set_status then get_status.
        let mut table = [0u8; 2];
        for idx in 0..4u32 {
            set_status(&mut table, 4, idx as usize);
            assert_eq!(
                get_status(&table, 4),
                idx as usize,
                "get_status should decode the index just encoded"
            );
        }
    }

    #[test]
    fn test_get_status_all_erased_is_zero() {
        // An all-erased table decodes to index 0 (Unused).
        let table = [0xFFu8; 2];
        assert_eq!(get_status(&table, 4), 0);
    }

    // ---- CRC32 ----

    /// Bitwise reference implementation of CRC-32/ISO-HDLC used to cross-check
    /// the table-driven version against the mathematical definition.
    fn crc32_reference(buf: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in buf {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        crc ^ 0xFFFF_FFFF
    }

    #[test]
    fn test_crc32_empty() {
        // c: fdb_calc_crc32(0, "", 0) == 0
        assert_eq!(calc_crc32(0, &[]), 0);
    }

    #[test]
    fn test_crc32_check_value() {
        // The canonical CRC-32 check value for "123456789" is 0xCBF43926.
        // FlashDB uses the standard CRC-32/ISO-HDLC algorithm, so this must match.
        assert_eq!(calc_crc32(0, b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn test_crc32_matches_reference() {
        // The table-driven implementation must agree with the bitwise reference
        // for a variety of inputs (this is the behavioural equivalence check
        // against the C version, whose table is identical).
        let inputs: &[&[u8]] = &[
            b"",
            b"Hello",
            b"FlashDB",
            b"123456789",
            &[0xFF; 4],
            &[0xFF; 32],
            &[0x00; 16],
            &(0u32..32).map(|x| x as u8).collect::<Vec<u8>>(),
        ];
        for inp in inputs {
            assert_eq!(
                calc_crc32(0, inp),
                crc32_reference(inp),
                "table CRC32 must match bitwise reference"
            );
        }
    }

    #[test]
    fn test_crc32_accumulation() {
        // Splitting the input and accumulating must equal the single-shot result.
        let data = b"FlashDB crc32 accumulation test";
        let whole = calc_crc32(0, data);
        let mut acc = 0u32;
        acc = calc_crc32(acc, &data[..8]);
        acc = calc_crc32(acc, &data[8..]);
        assert_eq!(acc, whole, "accumulated CRC must equal single-shot CRC");
    }

    // ---- flash I/O wrappers ----

    fn make_flash() -> MockFlash {
        MockFlash::new("test", 4096, 16384, 4096)
    }

    #[test]
    fn test_flash_read_write_erase() {
        let mut flash = make_flash();

        // erased flash reads as 0xFF
        let mut buf = [0u8; 4];
        flash_read(&flash, 0, &mut buf).unwrap();
        assert_eq!(buf, [0xFF; 4]);

        // write some bytes
        flash_write(&mut flash, 0, &[0x00, 0x01, 0x02, 0x03]).unwrap();
        flash_read(&flash, 0, &mut buf).unwrap();
        assert_eq!(buf, [0x00, 0x01, 0x02, 0x03]);

        // erase resets to 0xFF
        flash_erase(&mut flash, 0, 4096).unwrap();
        flash_read(&flash, 0, &mut buf).unwrap();
        assert_eq!(buf, [0xFF; 4]);
    }

    #[test]
    fn test_flash_write_align_gran1() {
        // For GRAN==1 the granularity is 1 byte, so write_align writes the
        // buffer verbatim with no padding.
        let mut flash = make_flash();
        flash_erase(&mut flash, 0, 4096).unwrap();
        flash_write_align(&mut flash, 0, &[0xAA, 0xBB, 0xCC]).unwrap();

        let mut buf = [0u8; 4];
        flash_read(&flash, 0, &mut buf).unwrap();
        assert_eq!(buf[..3], [0xAA, 0xBB, 0xCC], "first 3 bytes written verbatim");
        assert_eq!(buf[3], 0xFF, "untouched byte stays erased");
    }

    // Padding only occurs when the write granularity exceeds one byte
    // (GRAN >= 32). Under the default GRAN==1 this path is unreachable, so the
    // padding behaviour is verified here only when gran_64 is the *active*
    // granularity (i.e. no higher gran_* feature is enabled).
    #[cfg(all(
        feature = "gran_64",
        not(any(feature = "gran_128", feature = "gran_256"))
    ))]
    #[test]
    fn test_flash_write_align_padding_gran64() {
        let mut flash = make_flash();
        flash_erase(&mut flash, 0, 4096).unwrap();
        // GRAN==64 -> wg_bytes == 8. Writing 3 bytes pads to an 8-byte boundary.
        flash_write_align(&mut flash, 0, &[0xAA, 0xBB, 0xCC]).unwrap();

        let mut buf = [0u8; 8];
        flash_read(&flash, 0, &mut buf).unwrap();
        assert_eq!(buf[..3], [0xAA, 0xBB, 0xCC], "first 3 bytes are the payload");
        assert_eq!(buf[3..8], [0xFF; 5], "tail is padded with erased bytes");
    }

    // ---- write_status / read_status round-trip ----

    #[test]
    fn test_write_read_status_gran1() {
        let mut flash = make_flash();
        flash_erase(&mut flash, 0, 4096).unwrap();

        let mut table = [0u8; 2];
        for idx in 0..4u32 {
            // reset flash region for each iteration
            flash_erase(&mut flash, 0, 4096).unwrap();
            write_status(&mut flash, 0, &mut table, 4, idx as usize).unwrap();
            let decoded = read_status(&flash, 0, &mut table, 4);
            assert_eq!(
                decoded, idx as usize,
                "read_status must decode the index written by write_status"
            );
        }
    }

    #[test]
    fn test_write_status_index0_no_flash_write() {
        // index 0 means "all erased" -> set_status returns SIZE_MAX and
        // write_status must not touch flash. Prove this by pre-dirtying the
        // status byte with 0x00 and checking it is left untouched.
        let mut flash = make_flash();
        flash_erase(&mut flash, 0, 4096).unwrap();
        flash_write(&mut flash, 0, &[0x00]).unwrap();

        let mut table = [0u8; 2];
        write_status(&mut flash, 0, &mut table, 4, 0).unwrap();

        let mut buf = [0u8; 1];
        flash_read(&flash, 0, &mut buf).unwrap();
        assert_eq!(buf[0], 0x00, "index 0 must not modify flash (byte unchanged)");
        // The dirty 0x00 byte decodes as the fully-written status (index 3),
        // confirming read_status reports the on-flash contents, not index 0.
        assert_eq!(read_status(&flash, 0, &mut table, 4), 3);
    }

    // ---- continue_ff_addr ----

    #[test]
    fn test_continue_ff_addr_gran1() {
        let mut flash = make_flash();
        flash_erase(&mut flash, 0, 4096).unwrap();
        // Layout: [00 00 00 FF FF FF 00 FF FF]
        // C traces the last written->erased transition at offset 7, then since
        // the final byte is erased returns FDB_WG_ALIGN(7). For GRAN==1 that is 7.
        let data = [0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF];
        flash_write(&mut flash, 0, &data).unwrap();

        let addr = continue_ff_addr(&flash, 0, 9);
        assert_eq!(addr, 7, "GRAN==1: first contiguous-FF run starts at offset 7");
    }

    #[test]
    fn test_continue_ff_addr_all_written() {
        // When the whole range is written (no trailing 0xFF), the result is `end`.
        let mut flash = make_flash();
        flash_erase(&mut flash, 0, 4096).unwrap();
        flash_write(&mut flash, 0, &[0x00, 0x00, 0x00, 0x00]).unwrap();
        let addr = continue_ff_addr(&flash, 0, 4);
        assert_eq!(addr, 4, "no trailing 0xFF -> returns end");
    }

    #[test]
    fn test_continue_ff_addr_all_erased() {
        // When the whole range is erased, the transition (written->erased) never
        // occurs so `addr` stays at `start`; last_data is erased -> wg_align(start).
        let mut flash = make_flash();
        flash_erase(&mut flash, 0, 4096).unwrap();
        let addr = continue_ff_addr(&flash, 0, 32);
        assert_eq!(addr, wg_align(0), "all-erased range -> wg_align(start)");
    }

    // ---- blob API ----

    #[test]
    fn test_blob_make_defaults() {
        let mut buf = [0u8; 16];
        let blob = blob_make(&mut buf);
        assert_eq!(blob.size, 16);
        assert_eq!(blob.saved_meta_addr, FDB_DATA_UNUSED);
        assert_eq!(blob.saved_addr, FDB_FAILED_ADDR);
        assert_eq!(blob.saved_len, 0);
    }

    #[test]
    fn test_blob_read_full() {
        let mut flash = make_flash();
        let data: [u8; 32] = core::array::from_fn(|i| (i + 1) as u8); // [1..32]
        flash_write(&mut flash, 0, &data).unwrap();

        let mut buf = [0u8; 32];
        let mut blob = blob_make(&mut buf);
        blob.saved_addr = 0;
        blob.saved_len = 32;

        let read_len = blob_read(&flash, &mut blob);
        assert_eq!(read_len, 32, "full buffer read");
        assert_eq!(&buf[..32], &data[..], "read data matches written data");
    }

    #[test]
    fn test_blob_read_truncated() {
        // saved_len < buf.size -> read is truncated to saved_len.
        let mut flash = make_flash();
        flash_write(&mut flash, 0, &[0xAA; 16]).unwrap();

        let mut buf = [0u8; 32];
        let mut blob = blob_make(&mut buf);
        blob.saved_addr = 0;
        blob.saved_len = 16;

        let read_len = blob_read(&flash, &mut blob);
        assert_eq!(read_len, 16, "read truncated to saved_len");
        assert_eq!(&buf[..16], &[0xAA; 16], "first 16 bytes filled");
        assert_eq!(&buf[16..], &[0u8; 16], "tail untouched");
    }

    #[test]
    fn test_blob_read_bad_addr_returns_zero() {
        // Reading from an out-of-range address fails the flash read -> 0.
        let flash = make_flash();
        let mut buf = [0u8; 8];
        let mut blob = blob_make(&mut buf);
        // max_size is 16384; reading at an address that overflows device bounds
        blob.saved_addr = 16_380; // 16380 + 8 > 16384 -> out of bounds
        blob.saved_len = 8;
        let read_len = blob_read(&flash, &mut blob);
        assert_eq!(read_len, 0, "flash read failure yields 0 bytes");
    }
}
