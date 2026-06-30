// c: port/fal/fal_flash_dev — MockFlash in-memory flash simulation (test only)
//
// Simulates NOR flash behavior using a Vec<u8> buffer.
// Initial state: all 0xFF (erased).
// Write: can only 1→0 (NOR flash semantics).
// Erase: resets region to 0xFF.
//
// NOTE: This module is compiled unconditionally (not just under `cfg(test)`)
// so that integration tests in `tests/` can also use `MockFlash`.  It only
// depends on `alloc` (Vec), not `std`, so `no_std` compatibility is preserved.

use alloc::vec::Vec;
use alloc::vec;

use crate::def::{FdbErr, FDB_BYTE_ERASED};
use crate::flash_trait::FlashDevice;

/// Mock NOR flash device for testing.
///
/// Simulates a real NOR flash with:
/// - Initial state: all bytes 0xFF (erased)
/// - Write: AND operation (can only change 1→0)
/// - Erase: resets bytes to 0xFF
pub struct MockFlash {
    data: Vec<u8>,
    sec_size: u32,
    max_size: u32,
    block_size: u32,
    name: &'static str,
}

impl MockFlash {
    /// Create a new MockFlash with the given configuration.
    ///
    /// # Parameters
    /// - `name`: device name
    /// - `sec_size`: sector size in bytes
    /// - `max_size`: total device size in bytes (must be multiple of sec_size)
    /// - `block_size`: erase block size in bytes
    pub fn new(name: &'static str, sec_size: u32, max_size: u32, block_size: u32) -> Self {
        assert!(sec_size > 0, "sector size must be > 0");
        assert!(max_size > 0, "max size must be > 0");
        assert!(max_size % sec_size == 0, "max_size must be multiple of sec_size");
        assert!(sec_size % block_size == 0, "sec_size must be multiple of block_size");
        Self {
            data: vec![FDB_BYTE_ERASED; max_size as usize],
            sec_size,
            max_size,
            block_size,
            name,
        }
    }

    /// Get the sector size
    pub fn sec_size(&self) -> u32 {
        self.sec_size
    }

    /// Get the max size
    pub fn max_size(&self) -> u32 {
        self.max_size
    }

    /// Get the block size
    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    /// Get the device name
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Get a reference to the raw data (for test inspection)
    pub fn raw_data(&self) -> &[u8] {
        &self.data
    }
}

impl FlashDevice for MockFlash {
    fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(buf.len()).ok_or(FdbErr::ReadErr)?;
        if end > self.data.len() {
            return Err(FdbErr::ReadErr);
        }
        buf.copy_from_slice(&self.data[addr..end]);
        Ok(())
    }

    fn write(&mut self, addr: u32, buf: &[u8]) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(buf.len()).ok_or(FdbErr::WriteErr)?;
        if end > self.data.len() {
            return Err(FdbErr::WriteErr);
        }
        // NOR flash semantics: can only change 1→0 (AND operation)
        for (i, &byte) in buf.iter().enumerate() {
            self.data[addr + i] &= byte;
        }
        Ok(())
    }

    fn erase(&mut self, addr: u32, size: u32) -> Result<(), FdbErr> {
        let addr = addr as usize;
        let end = addr.checked_add(size as usize).ok_or(FdbErr::EraseErr)?;
        if end > self.data.len() {
            return Err(FdbErr::EraseErr);
        }
        self.data[addr..end].fill(FDB_BYTE_ERASED);
        Ok(())
    }

    fn len(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_flash_initial_erased() {
        let flash = MockFlash::new("test", 4096, 16384, 4096);
        let mut buf = [0u8; 16];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf, [0xFF; 16], "initial flash should be all 0xFF");
    }

    #[test]
    fn test_mock_flash_write_read() {
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        let data = [0x00, 0x01, 0x02, 0x03];
        flash.write(0, &data).unwrap();

        let mut buf = [0u8; 4];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf, data, "read should match written data");
    }

    #[test]
    fn test_mock_flash_nor_semantics_no_overwrite() {
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        // Write 0x00 (all bits to 0)
        flash.write(0, &[0x00]).unwrap();
        // Try to write 0xFF (should NOT restore to 1 — NOR flash can't 0→1 without erase)
        flash.write(0, &[0xFF]).unwrap();
        let mut buf = [0u8; 1];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf[0], 0x00, "NOR flash cannot change 0→1 without erase");
    }

    #[test]
    fn test_mock_flash_and_semantics() {
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        // Write 0xAA (10101010)
        flash.write(0, &[0xAA]).unwrap();
        // Write 0x55 (01010101) — AND = 0x00
        flash.write(0, &[0x55]).unwrap();
        let mut buf = [0u8; 1];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf[0], 0x00, "0xAA & 0x55 = 0x00 (NOR AND semantics)");
    }

    #[test]
    fn test_mock_flash_erase() {
        let mut flash = MockFlash::new("test", 4096, 16384, 4096);
        // Write some data
        flash.write(0, &[0x00, 0x01, 0x02, 0x03]).unwrap();
        // Erase
        flash.erase(0, 4096).unwrap();
        // Verify all 0xFF
        let mut buf = [0u8; 4];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf, [0xFF; 4], "after erase, should be all 0xFF");
    }

    #[test]
    fn test_mock_flash_read_out_of_bounds() {
        let flash = MockFlash::new("test", 4096, 4096, 4096);
        let mut buf = [0u8; 4];
        let result = flash.read(4094, &mut buf);
        assert_eq!(result, Err(FdbErr::ReadErr), "out of bounds read should fail");
    }

    #[test]
    fn test_mock_flash_write_out_of_bounds() {
        let mut flash = MockFlash::new("test", 4096, 4096, 4096);
        let result = flash.write(4094, &[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(result, Err(FdbErr::WriteErr), "out of bounds write should fail");
    }

    #[test]
    fn test_mock_flash_erase_out_of_bounds() {
        let mut flash = MockFlash::new("test", 4096, 4096, 4096);
        let result = flash.erase(0, 8192);
        assert_eq!(result, Err(FdbErr::EraseErr), "out of bounds erase should fail");
    }
}
