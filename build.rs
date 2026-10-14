//! 构建脚本：用 MinGW 的 windres 把 app.rc（exe 图标 + 版本信息）编译成 COFF 目标文件，
//! 再交给链接器。这样 exe 在资源管理器里显示自定义图标。
//!
//! 找不到 windres 时不中断构建（只是 exe 没有文件图标），并给出提示。

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_windres() -> Option<PathBuf> {
    // 1) 先看 PATH
    if let Ok(path) = env::var("PATH") {
        for dir in env::split_paths(&path) {
            for name in ["windres.exe", "windres"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    // 2) 常见 MinGW 安装位置
    let candidates = [
        r"D:\ProgramPackege\C++\mingw64\bin\windres.exe",
        r"C:\ProgramPackege\C++\mingw64\bin\windres.exe",
        r"C:\mingw64\bin\windres.exe",
        r"D:\Qt\Tools\mingw1120_64\bin\windres.exe",
        r"D:\Qt\Tools\mingw1310_64\bin\windres.exe",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/icons/app.ico");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    let rc_file = Path::new("app.rc");
    let res_obj = out_dir.join("app_res.o");

    let Some(windres) = find_windres() else {
        println!("cargo:warning=未找到 windres.exe，跳过图标资源编译（exe 将不带文件图标）");
        return;
    };

    let status = Command::new(&windres)
        .arg("--input-format=rc")
        .arg("--output-format=coff")
        .arg(format!("--include-dir={}", env::var("CARGO_MANIFEST_DIR").unwrap()))
        .arg("-i")
        .arg(rc_file)
        .arg("-o")
        .arg(&res_obj)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bins={}", res_obj.display());
        }
        Ok(s) => {
            println!("cargo:warning=windres 编译 app.rc 失败（退出码 {s}），exe 将不带文件图标");
        }
        Err(err) => {
            println!("cargo:warning=调用 windres 失败：{err}，exe 将不带文件图标");
        }
    }
}
