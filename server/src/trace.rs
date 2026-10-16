//! 启动追踪：仅在 `--selftest` 模式下启用，把关键步骤写入日志文件。
//!
//! 图形程序出错时窗口不会出现、控制台也没有输出，排查很困难；
//! 这个模式让程序自己把"走到哪一步"落盘。

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

static SINK: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

pub fn init(path: &str) {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok();
    let _ = SINK.set(Mutex::new(file));
    log("=== selftest start ===");
}

pub fn enabled() -> bool {
    SINK.get().is_some()
}

pub fn log(msg: &str) {
    if let Some(sink) = SINK.get() {
        if let Ok(mut guard) = sink.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = writeln!(file, "{}", msg);
                let _ = file.flush();
            }
        }
    }
}

#[macro_export]
macro_rules! tlog {
    ($($arg:tt)*) => {
        if $crate::trace::enabled() {
            $crate::trace::log(&format!($($arg)*))
        }
    };
}
