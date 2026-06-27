# FAL vtable → trait FlashDevice

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 用函数指针表（vtable）实现 flash 设备抽象：

```c
// C: FAL flash 设备 vtable
struct fal_flash_dev {
    char name[24];
    uint32_t addr;
    size_t len;
    size_t blk_size;
    struct {
        int (*init)(void);
        int (*read)(long offset, uint8_t *buf, size_t size);
        int (*write)(long offset, const uint8_t *buf, size_t size);
        int (*erase)(long offset, size_t size);
    } ops;
    size_t write_gran;
};

// 静态注册设备
const struct fal_flash_dev stm32_onchip_flash = {
    .name = "stm32_onchip",
    .addr = 0x08000000,
    .len = 256*1024,
    .blk_size = 2*1024,
    .ops = {init, read, write, erase},
    .write_gran = 32,
};

// 运行时通过函数指针调用
int fal_partition_read(const struct fal_partition *part, uint32_t addr, uint8_t *buf, size_t size) {
    const struct fal_flash_dev *flash_dev = part->flash_dev;
    return flash_dev->ops.read(part->offset + addr, buf, size);
}
```

## 解决方案

### 方案 A：Rust trait + 静态分发（推荐，零开销）

```rust
// 定义 flash 设备 trait
pub trait FlashDevice {
    /// 初始化设备
    fn init(&mut self) -> Result<(), FlashError>;

    /// 读取数据，返回实际读取字节数
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, FlashError>;

    /// 写入数据，返回实际写入字节数
    fn write(&mut self, offset: u32, buf: &[u8]) -> Result<usize, FlashError>;

    /// 擦除指定区域
    fn erase(&mut self, offset: u32, size: u32) -> Result<u32, FlashError>;

    /// 设备名称
    fn name(&self) -> &str;

    /// 设备起始地址
    fn addr(&self) -> u32;

    /// 设备总大小
    fn len(&self) -> usize;

    /// 块大小（擦除单元）
    fn blk_size(&self) -> usize;

    /// 写粒度（1/8/32/64/128/256 bit）
    fn write_gran(&self) -> u32;
}

// STM32F4 内部 flash 实现
pub struct Stm32F4Flash {
    base_addr: u32,
    total_size: usize,
    sector_size: usize,
}

impl FlashDevice for Stm32F4Flash {
    fn init(&mut self) -> Result<(), FlashError> {
        // STM32 HAL 初始化
        Ok(())
    }

    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, FlashError> {
        let addr = self.base_addr + offset;
        // 内存映射 flash 读取——必须用 read_volatile
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = unsafe { core::ptr::read_volatile((addr + i as u32) as *const u8) };
        }
        Ok(buf.len())
    }

    fn write(&mut self, offset: u32, buf: &[u8]) -> Result<usize, FlashError> {
        // STM32 HAL 写入
        // unsafe { hal_flash_program(addr, buf) };
        Ok(buf.len())
    }

    fn erase(&mut self, offset: u32, size: u32) -> Result<u32, FlashError> {
        // STM32 HAL 擦除
        Ok(size)
    }

    fn name(&self) -> &str { "stm32_onchip" }
    fn addr(&self) -> u32 { self.base_addr }
    fn len(&self) -> usize { self.total_size }
    fn blk_size(&self) -> usize { self.sector_size }
    fn write_gran(&self) -> u32 { 8 }  // STM32F4 = 8 bit
}
```

### 方案 B：`embedded-storage` trait 适配

```rust
// 适配 embedded-storage 的 NorFlash trait
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};

impl NorFlash for Stm32F4Flash {
    const WRITE_SIZE: usize = 1;
    const ERASE_SIZE: usize = 2048;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        FlashDevice::erase(self, from, to - from).map(|_| ())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        FlashDevice::write(self, offset, bytes).map(|_| ())
    }
}

impl ReadNorFlash for Stm32F4Flash {
    const READ_SIZE: usize = 1;

    fn read(&self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        FlashDevice::read(self, offset, bytes).map(|_| ())
    }

    fn capacity(&self) -> usize {
        self.total_size
    }
}
```

### 方案 C：分区抽象

```rust
// 分区是 flash 设备上的一段区域
pub struct Partition<F: FlashDevice> {
    flash: F,
    name: &'static str,
    offset: u32,
    len: u32,
}

impl<F: FlashDevice> Partition<F> {
    pub fn new(flash: F, name: &'static str, offset: u32, len: u32) -> Self {
        Self { flash, name, offset, len }
    }

    pub fn read(&self, addr: u32, buf: &mut [u8]) -> Result<usize, FlashError> {
        self.flash.read(self.offset + addr, buf)
    }

    pub fn write(&mut self, addr: u32, buf: &[u8]) -> Result<usize, FlashError> {
        self.flash.write(self.offset + addr, buf)
    }

    pub fn erase(&mut self, addr: u32, size: u32) -> Result<u32, FlashError> {
        self.flash.erase(self.offset + addr, size)
    }
}

// 设备表（替代 C 的 device_table[]）
pub struct FlashDeviceRegistry {
    devices: &'static [&'static dyn ErasedFlashDevice],
}

// 类型擦除的 trait object（用于设备表）
pub trait ErasedFlashDevice: FlashDevice + Sync {
    fn as_flash_device(&self) -> &dyn FlashDevice;
}

// 注册设备
static DEVICE_TABLE: &[&dyn ErasedFlashDevice] = &[
    &Stm32F4Flash { base_addr: 0x08000000, total_size: 1024 * 1024, sector_size: 128 * 1024 },
    // &NorFlash0 { ... },
];
```

## Rust-for-Linux 模式参考

RfL 用 `#[vtable]` 宏将 C vtable 转为 trait，然后生成静态 C 兼容的函数指针表：

```rust
// RfL 风格：trait 定义行为
#[vtable]
pub trait FlashOperations: Sized {
    fn init(&mut self) -> Result<(), FlashError>;
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, FlashError>;
    fn write(&mut self, offset: u32, buf: &[u8]) -> Result<usize, FlashError>;
    fn erase(&mut self, offset: u32, size: u32) -> Result<u32, FlashError>;
}

// 生成 HAS_* 常量检测哪些方法被实现
// 然后构建静态 vtable 用于 C FFI
```

## ❌ 禁止模式

```rust
// 禁止：用函数指针 struct 保持 C vtable 模式
struct FlashOps {
    init: Option<extern "C" fn() -> i32>,
    read: Option<extern "C" fn(u32, *mut u8, usize) -> i32>,
    // ...
}
// 原因：失去 Rust 类型安全；应改用 trait

// 禁止：用 Box<dyn FlashDevice> 存储所有设备（除非需要运行时多态）
// 原因：动态分发有性能开销；静态分发用泛型参数更优
```

## 注意事项

- `trait FlashDevice` 是静态分发（泛型），零运行时开销
- `&dyn FlashDevice` 是动态分发（trait object），用于设备表
- `embedded-storage` 是 Rust 嵌入式生态标准 trait，优先适配
- 内存映射 flash 读取必须用 `read_volatile`（C 代码缺少 volatile 是 bug）
- 设备表用 `&'static [&'static dyn ErasedFlashDevice]`，编译时确定
