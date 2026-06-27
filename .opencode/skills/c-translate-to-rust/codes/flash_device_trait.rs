// flash_device_trait.rs — FlashDevice trait + embedded-storage 适配
// 生产级参考实现，可直接复制使用
//
// 返回 [SKILL.md](../SKILL.md) references/fal-to-trait.md

#![cfg_attr(not(feature = "std"), no_std)]

use core::fmt;

/// Flash 设备错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashError {
    /// 读取错误
    ReadError,
    /// 写入错误
    WriteError,
    /// 擦除错误
    EraseError,
    /// 地址越界
    OutOfBounds,
    /// 对齐错误
    Misaligned,
    /// 验证失败（写后读取不一致）
    VerifyFailed,
    /// 设备未初始化
    NotInitialized,
    /// 设备已忙
    Busy,
}

impl fmt::Display for FlashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlashError::ReadError => write!(f, "flash read error"),
            FlashError::WriteError => write!(f, "flash write error"),
            FlashError::EraseError => write!(f, "flash erase error"),
            FlashError::OutOfBounds => write!(f, "address out of bounds"),
            FlashError::Misaligned => write!(f, "misaligned address"),
            FlashError::VerifyFailed => write!(f, "write verification failed"),
            FlashError::NotInitialized => write!(f, "flash device not initialized"),
            FlashError::Busy => write!(f, "flash device busy"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for FlashError {}

/// Flash 设备 trait —— 替代 C 的 `struct fal_flash_dev` + 函数指针 vtable
///
/// 每个 flash 设备实现此 trait，提供 read/write/erase 操作。
/// 参考 Rust-for-Linux 的 `#[vtable]` 模式：trait 定义行为，实现提供具体逻辑。
pub trait FlashDevice {
    /// 初始化设备
    fn init(&mut self) -> Result<(), FlashError>;

    /// 读取数据到 buf，返回实际读取字节数
    ///
    /// # 参数
    /// - `offset`: 相对于设备起始地址的偏移
    /// - `buf`: 读取缓冲区
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, FlashError>;

    /// 写入数据，返回实际写入字节数
    ///
    /// # 参数
    /// - `offset`: 相对于设备起始地址的偏移
    /// - `buf`: 待写入数据
    fn write(&mut self, offset: u32, buf: &[u8]) -> Result<usize, FlashError>;

    /// 擦除指定区域
    ///
    /// # 参数
    /// - `offset`: 起始偏移
    /// - `size`: 擦除大小（必须是 blk_size 的倍数）
    fn erase(&mut self, offset: u32, size: u32) -> Result<u32, FlashError>;

    /// 设备名称
    fn name(&self) -> &str;

    /// 设备起始地址（内存映射 flash 的物理地址）
    fn addr(&self) -> u32;

    /// 设备总大小（字节）
    fn len(&self) -> usize;

    /// 设备是否为空
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 块大小（擦除单元，字节）
    fn blk_size(&self) -> usize;

    /// 写粒度（bit）：1=NOR flash, 8=STM32F2/F4, 32=STM32F1, 64=STM32L4, 128=STM32H5, 256=STM32H7
    fn write_gran(&self) -> u32;
}

/// 分区：flash 设备上的一段区域
///
/// 替代 C 的 `struct fal_partition`
pub struct Partition<F: FlashDevice> {
    flash: F,
    name: &'static str,
    offset: u32,
    len: u32,
}

impl<F: FlashDevice> Partition<F> {
    /// 创建分区
    pub fn new(flash: F, name: &'static str, offset: u32, len: u32) -> Self {
        Self { flash, name, offset, len }
    }

    /// 分区名称
    pub fn name(&self) -> &str {
        self.name
    }

    /// 分区起始偏移
    pub fn offset(&self) -> u32 {
        self.offset
    }

    /// 分区大小
    pub fn len(&self) -> u32 {
        self.len
    }

    /// 分区是否为空
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 读取数据
    pub fn read(&self, addr: u32, buf: &mut [u8]) -> Result<usize, FlashError> {
        if addr + buf.len() as u32 > self.len {
            return Err(FlashError::OutOfBounds);
        }
        self.flash.read(self.offset + addr, buf)
    }

    /// 写入数据
    pub fn write(&mut self, addr: u32, buf: &[u8]) -> Result<usize, FlashError> {
        if addr + buf.len() as u32 > self.len {
            return Err(FlashError::OutOfBounds);
        }
        self.flash.write(self.offset + addr, buf)
    }

    /// 擦除指定区域
    pub fn erase(&mut self, addr: u32, size: u32) -> Result<u32, FlashError> {
        if addr + size > self.len {
            return Err(FlashError::OutOfBounds);
        }
        self.flash.erase(self.offset + addr, size)
    }

    /// 获取内部 flash 设备引用
    pub fn flash(&self) -> &F {
        &self.flash
    }

    /// 获取内部 flash 设备可变引用
    pub fn flash_mut(&mut self) -> &mut F {
        &mut self.flash
    }
}

// ====== embedded-storage 适配（可选） ======

#[cfg(feature = "embedded-storage")]
mod embedded_storage_adapter {
    use super::*;
    use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

    /// 为实现了 FlashDevice 的类型自动实现 NorFlash
    /// 注意：需要具体类型手动 impl，这里展示模式
    pub trait FlashDeviceExt: FlashDevice + Sized {
        const READ_SIZE: usize = 1;
        const WRITE_SIZE: usize = 1;
        const ERASE_SIZE: usize = 4096;
    }

    // 示例：为 Stm32F4Flash 实现 NorFlash
    // impl NorFlash for Stm32F4Flash {
    //     const WRITE_SIZE: usize = 1;
    //     const ERASE_SIZE: usize = 2048;
    //     fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
    //         FlashDevice::erase(self, from, to - from).map(|_| ())
    //     }
    //     fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
    //         FlashDevice::write(self, offset, bytes).map(|_| ())
    //     }
    // }
}

// ====== 内存映射 flash 读取（必须用 read_volatile） ======

/// 安全的内存映射 flash 读取
///
/// C 代码 `*buf = *(uint8_t *)addr;` 缺少 volatile 是 bug。
/// Rust 必须用 `ptr::read_volatile`。
pub fn read_mmio(addr: u32, buf: &mut [u8]) {
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((addr + i as u32) as *const u8) };
    }
}

/// 安全的内存映射 flash 读取（32 位）
pub fn read_mmio_u32(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

// ====== RAII Lock Guard（替代 C 的 HAL_FLASH_Unlock/Lock） ======

/// Flash 锁守卫，Drop 时自动加锁
pub struct FlashLockGuard<'a, F: FlashDevice> {
    flash: &'a mut F,
    unlock_fn: fn(),
    lock_fn: fn(),
}

impl<'a, F: FlashDevice> FlashLockGuard<'a, F> {
    /// 创建守卫，立即解锁
    pub fn new(flash: &'a mut F, unlock_fn: fn(), lock_fn: fn()) -> Self {
        unlock_fn();
        Self { flash, unlock_fn, lock_fn }
    }

    /// 获取 flash 可变引用
    pub fn flash(&mut self) -> &mut F {
        self.flash
    }
}

impl<F: FlashDevice> Drop for FlashLockGuard<'_, F> {
    fn drop(&mut self) {
        (self.lock_fn)();
    }
}

// ====== 设备注册表（替代 C 的 device_table[]） ======

/// 类型擦除的 flash 设备 trait（用于设备表）
pub trait ErasedFlashDevice: FlashDevice + Sync {
    fn as_flash_device(&self) -> &dyn FlashDevice;
}

// 为所有满足约束的类型自动实现
impl<T: FlashDevice + Sync> ErasedFlashDevice for T {
    fn as_flash_device(&self) -> &dyn FlashDevice {
        self
    }
}

/// 设备注册表
pub struct DeviceRegistry {
    devices: &'static [&'static dyn ErasedFlashDevice],
}

impl DeviceRegistry {
    /// 创建注册表
    pub const fn new(devices: &'static [&'static dyn ErasedFlashDevice]) -> Self {
        Self { devices }
    }

    /// 按名称查找设备
    pub fn find(&self, name: &str) -> Option<&'static dyn FlashDevice> {
        self.devices.iter()
            .find(|dev| dev.as_flash_device().name() == name)
            .map(|dev| dev.as_flash_device())
    }

    /// 获取所有设备
    pub fn devices(&self) -> &[&'static dyn ErasedFlashDevice] {
        self.devices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock flash 设备（用于测试）
    pub struct MockFlash {
        data: Vec<u8>,
        name: &'static str,
    }

    impl MockFlash {
        pub fn new(name: &'static str, size: usize) -> Self {
            Self {
                data: vec![0xFF; size],
                name,
            }
        }
    }

    impl FlashDevice for MockFlash {
        fn init(&mut self) -> Result<(), FlashError> {
            Ok(())
        }

        fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, FlashError> {
            let offset = offset as usize;
            if offset + buf.len() > self.data.len() {
                return Err(FlashError::OutOfBounds);
            }
            buf.copy_from_slice(&self.data[offset..offset + buf.len()]);
            Ok(buf.len())
        }

        fn write(&mut self, offset: u32, buf: &[u8]) -> Result<usize, FlashError> {
            let offset = offset as usize;
            if offset + buf.len() > self.data.len() {
                return Err(FlashError::OutOfBounds);
            }
            // NOR flash 语义：只能 1→0
            for (i, byte) in buf.iter().enumerate() {
                self.data[offset + i] &= byte;  // AND 操作模拟 flash 写入
            }
            Ok(buf.len())
        }

        fn erase(&mut self, offset: u32, size: u32) -> Result<u32, FlashError> {
            let offset = offset as usize;
            let size = size as usize;
            if offset + size > self.data.len() {
                return Err(FlashError::OutOfBounds);
            }
            self.data[offset..offset + size].fill(0xFF);
            Ok(size as u32)
        }

        fn name(&self) -> &str { self.name }
        fn addr(&self) -> u32 { 0 }
        fn len(&self) -> usize { self.data.len() }
        fn blk_size(&self) -> usize { 4096 }
        fn write_gran(&self) -> u32 { 1 }
    }

    #[test]
    fn test_mock_flash_write_read() {
        let mut flash = MockFlash::new("mock", 4096);

        // 写入
        let data = [0xAA, 0xBB, 0xCC, 0xDD];
        flash.write(0, &data).unwrap();

        // 读取验证
        let mut buf = [0u8; 4];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf, data);
    }

    #[test]
    fn test_mock_flash_erase() {
        let mut flash = MockFlash::new("mock", 4096);

        // 写入
        flash.write(0, &[0x12, 0x34]).unwrap();

        // 擦除
        flash.erase(0, 4096).unwrap();

        // 验证擦除后为 0xFF
        let mut buf = [0u8; 2];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf, [0xFF, 0xFF]);
    }

    #[test]
    fn test_nor_flash_semantics() {
        let mut flash = MockFlash::new("mock", 16);

        // 初始全 0xFF
        let mut buf = [0u8; 4];
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf, [0xFF, 0xFF, 0xFF, 0xFF]);

        // 写入 0xAA（1→0 转换）
        flash.write(0, &[0xAA]).unwrap();
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf[0], 0xAA);

        // 再写 0x55（只能 1→0，0xAA & 0x55 = 0x00）
        flash.write(0, &[0x55]).unwrap();
        flash.read(0, &mut buf).unwrap();
        assert_eq!(buf[0], 0x00);  // 0xAA & 0x55 = 0x00
    }

    #[test]
    fn test_partition() {
        let flash = MockFlash::new("mock", 4096);
        let mut partition = Partition::new(flash, "test_part", 1024, 2048);

        // 写入分区
        partition.write(0, &[1, 2, 3]).unwrap();

        // 读取分区
        let mut buf = [0u8; 3];
        partition.read(0, &mut buf).unwrap();
        assert_eq!(buf, [1, 2, 3]);
    }

    #[test]
    fn test_partition_out_of_bounds() {
        let flash = MockFlash::new("mock", 4096);
        let mut partition = Partition::new(flash, "test_part", 0, 1024);

        // 越界写入
        let result = partition.write(1024, &[1]);
        assert_eq!(result, Err(FlashError::OutOfBounds));

        // 越界读取
        let mut buf = [0u8; 1];
        let result = partition.read(1024, &mut buf);
        assert_eq!(result, Err(FlashError::OutOfBounds));
    }
}
