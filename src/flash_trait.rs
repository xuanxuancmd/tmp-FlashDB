// c: port/fal/fal.h — FlashDevice trait definition
//
// Replaces C's FAL `struct fal_flash_dev` + function pointer vtable
// with a Rust trait. See skill references/fal-to-trait.md.

use crate::def::FdbErr;

/// c: port/fal/fal_flash_dev — Flash device abstraction trait.
///
/// Each flash device implements this trait to provide read/write/erase operations.
/// Replaces C's FAL vtable (`struct fal_flash_dev { int (*read)(...); ... }`).
///
/// NOR flash semantics:
/// - `write`: can only change bits from 1→0 (no 0→1 without erase first)
/// - `erase`: sets all bits in the region to 1 (0xFF)
pub trait FlashDevice {
    /// c: fal_partition_read — read data from flash
    ///
    /// # Parameters
    /// - `addr`: offset from device start
    /// - `buf`: destination buffer
    fn read(&self, addr: u32, buf: &mut [u8]) -> Result<(), FdbErr>;

    /// c: fal_partition_write — write data to flash
    ///
    /// NOR flash constraint: can only change 1→0 bits.
    /// To write 0→1, must erase first.
    ///
    /// # Parameters
    /// - `addr`: offset from device start
    /// - `buf`: source data
    fn write(&mut self, addr: u32, buf: &[u8]) -> Result<(), FdbErr>;

    /// c: fal_partition_erase — erase a region of flash
    ///
    /// Sets all bytes in [addr, addr+size) to 0xFF.
    ///
    /// # Parameters
    /// - `addr`: start offset (must be aligned to block size)
    /// - `size`: bytes to erase (must be multiple of block size)
    fn erase(&mut self, addr: u32, size: u32) -> Result<(), FdbErr>;

    /// Total device size in bytes
    fn len(&self) -> usize;

    /// Whether the device is empty (size == 0)
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
