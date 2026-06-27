# 内存映射 flash 读取 → read_volatile

> 返回 [SKILL.md](../SKILL.md) §1-B

## 问题

C 代码直接通过指针解引用读取内存映射 flash，**缺少 `volatile` 关键字**（潜在 bug）：

```c
// C: STM32 HAL flash 读取——缺少 volatile
static int read(long offset, uint8_t *buf, size_t size) {
    uint32_t addr = 0x08000000 + offset;
    for (size_t i = 0; i < size; i++, addr++, buf++) {
        *buf = *(uint8_t *)addr;  // ← 缺少 volatile！编译器可能优化掉
    }
    return size;
}

// C: 写后验证读取
read_data = *(uint32_t *)addr;  // 同样缺少 volatile
if (read_data != write_data) {
    return -1;
}
```

C 编译器可能优化掉这些读取（因为看起来没有副作用）。正确写法应该是 `*(volatile uint8_t *)addr`，但 FlashDB 的移植代码都没加。

## 解决方案

### 方案 A：`ptr::read_volatile`（推荐）

```rust
// Rust: 用 read_volatile 读取内存映射 flash
use core::ptr;

impl FlashDevice for Stm32F4Flash {
    fn read(&self, offset: u32, buf: &mut [u8]) -> Result<usize, FlashError> {
        let addr = self.base_addr + offset;
        for (i, byte) in buf.iter_mut().enumerate() {
            // read_volatile 防止编译器优化掉读取
            *byte = unsafe { ptr::read_volatile((addr + i as u32) as *const u8) };
        }
        Ok(buf.len())
    }
}

// 写后验证
fn write_and_verify(flash: &mut impl FlashDevice, addr: u32, data: &[u8]) -> Result<(), FlashError> {
    flash.write(addr, data)?;

    // 验证：用 read_volatile 读取（不能用普通指针解引用）
    let mut read_back = [0u8; 4];
    for (i, byte) in read_back.iter_mut().enumerate() {
        *byte = unsafe { ptr::read_volatile((addr + i as u32) as *const u8) };
    }

    if read_back != data {
        return Err(FlashError::VerifyFailed);
    }
    Ok(())
}
```

### 方案 B：`slice::from_raw_parts` + volatile 读取

```rust
// 批量 volatile 读取
fn read_volatile_slice(base_addr: u32, offset: u32, buf: &mut [u8]) {
    let addr = base_addr + offset;
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte = unsafe { ptr::read_volatile((addr + i as u32) as *const u8) };
    }
}

// 对于对齐的批量读取（32 位）
fn read_volatile_u32(addr: u32) -> u32 {
    unsafe { ptr::read_volatile(addr as *const u32) }
}
```

### 方案 C：`volatile-register` crate（寄存器级别）

```rust
// 对寄存器级别的 volatile 访问，用 volatile-register crate
use volatile_register::{RO, RW};

// 只读寄存器
const FLASH_SR: *const RO<u32> = 0x40022000 as *const RO<u32>;

// 读写寄存器
const FLASH_CR: *mut RW<u32> = 0x40022010 as *mut RW<u32>;

fn read_flash_status() -> u32 {
    unsafe { (*FLASH_SR).read() }
}
```

## flash 写入的对齐要求

不同 STM32 系列有不同的写粒度：

```rust
// STM32F1: 32 位写
fn write_word(addr: u32, data: u32) -> Result<(), FlashError> {
    // HAL_FLASH_Program(FLASH_TYPEPROGRAM_WORD, addr, data)
    unsafe { hal_flash_program_word(addr, data) };
    Ok(())
}

// STM32F4: 8 位写（字节编程）
fn write_byte(addr: u32, data: u8) -> Result<(), FlashError> {
    unsafe { hal_flash_program_byte(addr, data) };
    Ok(())
}

// STM32L4: 64 位写（双字编程）
fn write_doubleword(addr: u32, data: u64) -> Result<(), FlashError> {
    // 跳过全 0xFF 的写入（flash 特性：只能 1→0）
    if data != 0xFFFF_FFFF_FFFF_FFFF {
        unsafe { hal_flash_program_doubleword(addr, data) };
    }
    Ok(())
}
```

## RAII Lock Guard

C 用 `HAL_FLASH_Unlock()` / `HAL_FLASH_Lock()` 手动管理 flash 解锁状态：

```c
// C: 手动 unlock/lock
HAL_FLASH_Unlock();
// ... 写入操作 ...
HAL_FLASH_Lock();
```

Rust 用 RAII 自动管理：

```rust
struct FlashLockGuard<'a> {
    flash: &'a mut Stm32F4Flash,
}

impl<'a> FlashLockGuard<'a> {
    fn new(flash: &'a mut Stm32F4Flash) -> Self {
        unsafe { hal_flash_unlock() };
        Self { flash }
    }
}

impl Drop for FlashLockGuard<'_> {
    fn drop(&mut self) {
        unsafe { hal_flash_lock() };
    }
}

// 使用：锁在作用域结束时自动释放
impl FlashDevice for Stm32F4Flash {
    fn write(&mut self, offset: u32, buf: &[u8]) -> Result<usize, FlashError> {
        let _lock = FlashLockGuard::new(self);  // 自动 unlock

        for (i, byte) in buf.iter().enumerate() {
            unsafe { hal_flash_program_byte(self.base_addr + offset + i as u32, *byte) };
            // 写后验证
            let read_back = unsafe { ptr::read_volatile((self.base_addr + offset + i as u32) as *const u8) };
            if read_back != *byte {
                return Err(FlashError::VerifyFailed);  // _lock drop → 自动 lock
            }
        }
        Ok(buf.len())  // _lock drop → 自动 lock
    }
}
```

## ❌ 禁止模式

```rust
// 禁止：普通指针解引用读取 flash（编译器可能优化掉）
let byte = unsafe { *(addr as *const u8) };

// 禁止：用 AtomicU32 读取 flash（atomic 语义不适用于 MMIO）
let val = AtomicU32::new(0);
val.load(Ordering::SeqCst);  // 不是 volatile 读取

// 禁止：std::hint::black_box 替代 volatile（不保证不被优化）
let byte = std::hint::black_box(unsafe { *(addr as *const u8) });
```

## 注意事项

- C 代码缺少 `volatile` 是**已知 bug**——Rust 翻译时必须修复
- `read_volatile` / `write_volatile` 是 Rust 中唯一安全的 MMIO 读取方式
- `embedded-storage` trait 已经封装了 volatile 读取，优先使用
- flash 写入只能 1→0（NOR flash 特性），全 0xFF 的写入可跳过
- 写后验证是必要的——flash 写入可能因硬件错误失败
