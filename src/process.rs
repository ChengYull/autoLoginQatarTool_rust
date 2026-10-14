//! 进程检测：一次 Toolhelp32 快照判断目标客户端是否在运行。
//!
//! 原版每轮起一个 `tasklist` 子进程同步等待，实测单次约 200ms 且阻塞界面线程；
//! 这里沿用 C++ 版后期的做法 —— 一次系统调用，约 7~9ms，不创建子进程。

use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

use crate::util::{contains_ignore_case, from_wide};

/// 需要监测的客户端进程映像名
const TARGET_IMAGES: [&str; 2] = ["QaTar.exe", "QtPacker.exe"];

/// 是否为需要监测的目标进程：先按映像名精确匹配（不区分大小写），
/// 再按关键字 `qtpacker` 兜底，兼容带版本号或改名后的映像名。
fn is_target_process(image_name: &str) -> bool {
    TARGET_IMAGES
        .iter()
        .any(|target| image_name.eq_ignore_ascii_case(target))
        || contains_ignore_case(image_name, "qtpacker")
}

/// 通过系统进程快照查找目标进程
pub fn is_target_running() -> bool {
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        if snapshot.is_invalid() {
            return false;
        }

        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut found = false;
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name: String = from_wide(&entry.szExeFile[..MAX_PATH as usize]);
                if is_target_process(&name) {
                    found = true;
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
        found
    }
}
