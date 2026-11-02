//! 构建脚本：用 MinGW 的 windres 把 app.rc（exe 图标 + 版本信息）编译成 COFF 目标文件，
//! 再交给链接器。这样 exe 在资源管理器里显示自定义图标。
//!
//! app.rc 里的版本号写成占位符（`@VERSION_STR@` / `@VERSION_COMMA@`），
//! 由这里从 `CARGO_PKG_VERSION` 填进去 —— 版本号只在 Cargo.toml 里维护一份，
//! 界面徽章（`version.rs`）和 exe 属性（资源里的 VERSIONINFO）不会再对不上。
//!
//! 找不到 windres 时不中断构建（只是 exe 没有文件图标），并给出提示。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 把 `1.0.1` 拆成资源脚本要的 `1,0,1,0`（不足 4 段补 0）。
/// VERSIONINFO 的 FILEVERSION 是 4 个 16 位整数，少一段 windres 会报错。
fn version_comma(version: &str) -> String {
    let mut parts: Vec<u16> = version
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u16>().unwrap_or(0))
        .collect();
    parts.resize(4, 0);
    parts
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

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
    let res_obj = out_dir.join("app_res.o");

    // 先把 app.rc 里的版本占位符换成 Cargo.toml 里的真实版本，产物落到 OUT_DIR，
    // 不污染仓库里的源文件。
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION 未设置");
    let template = match fs::read_to_string(Path::new(&manifest_dir).join("app.rc")) {
        Ok(t) => t,
        Err(err) => {
            println!("cargo:warning=读取 app.rc 失败：{err}，exe 将不带文件图标");
            return;
        }
    };
    let rc_file = out_dir.join("app.resolved.rc");
    let resolved = template
        .replace("@VERSION_STR@", &version)
        .replace("@VERSION_COMMA@", &version_comma(&version));
    if let Err(err) = fs::write(&rc_file, resolved) {
        println!("cargo:warning=写出 app.resolved.rc 失败：{err}，exe 将不带文件图标");
        return;
    }

    let Some(windres) = find_windres() else {
        println!("cargo:warning=未找到 windres.exe，跳过图标资源编译（exe 将不带文件图标）");
        return;
    };

    let status = Command::new(&windres)
        .arg("--input-format=rc")
        .arg("--output-format=coff")
        // app.rc 是 UTF-8（里面有中文的 FileDescription）。不指定代码页时 windres
        // 按系统 ANSI（本机 936/GBK）读，UTF-8 的中文会被当成 GBK 双字节解一次，
        // 结果 exe 属性里的「文件说明」是一串乱码（1.0.4 ~ 1.0.5 都是这样）。
        .arg("--codepage=65001")
        .arg(format!("--include-dir={manifest_dir}"))
        .arg("-i")
        .arg(&rc_file)
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
