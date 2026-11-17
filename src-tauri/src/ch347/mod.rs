#[cfg(windows)]
pub mod commands;
#[cfg(not(windows))]
#[path = "commands_stub.rs"]
pub mod commands;
pub mod device_notifier;

use std::collections::HashMap;
use std::sync::Mutex;

/// 设备打开状态：index → 引用计数。
///
/// 用引用计数而非单纯的"是否打开"列表，是为了支持前端工具与 Python 脚本（经 HTTP 服务）
/// 共享同一物理设备：同一 index 可被多次 open（计数 +1），close 时计数 -1，归零才真正调
/// DLL 的 `CH347CloseDevice`。否则会出现：
///   1. 设备已被前端打开，脚本再 open → 对已打开设备重复调 DLL open → 破坏 DLL 内部状态
///   2. 脚本结束 close → 把前端还在用的设备关掉 → 后续扫描操作命中"半死"状态 → DLL 崩溃闪退
pub struct Ch347State {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub open_count: Mutex<HashMap<u32, u32>>,
    /// Serialize native/DLL access. The adapter exposes one shared command pipe,
    /// so overlapping scans and GPIO transactions can corrupt replies.
    #[allow(dead_code)]
    pub io_lock: Mutex<()>,
    pub dll_available: bool,
}

impl Ch347State {
    pub fn new() -> Self {
        #[cfg(windows)]
        // Try 64-bit DLL first, then 32-bit
        let dll_available = unsafe {
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!(
                "CH347DLLA64.dll"
            ))
            .is_ok()
                || windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!(
                    "CH347DLL.dll"
                ))
                .is_ok()
        };
        #[cfg(not(windows))]
        let dll_available = true;
        if dll_available {
            log::info!("CH347 hardware backend available");
        } else {
            log::info!("CH347 DLL not found — I2C/SPI hardware unavailable");
        }
        Self {
            open_count: Mutex::new(HashMap::new()),
            io_lock: Mutex::new(()),
            dll_available,
        }
    }

    #[allow(dead_code)]
    pub fn is_available(&self) -> bool {
        self.dll_available
    }
}
