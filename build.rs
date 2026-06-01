//! Embeds `assets/icon.ico` into the Windows executable via a resource script.
//! Uses the SDK `rc.exe` only — no build-dependencies.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    #[cfg(windows)]
    embed_windows_icon();
}

#[cfg(windows)]
fn embed_windows_icon() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let assets = manifest_dir.join("assets");
    let rc_src = assets.join("trayvault.rc");
    let res_out = out_dir.join("trayvault.res");

    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/trayvault.rc");

    if !rc_src.is_file() {
        panic!("missing {}", rc_src.display());
    }

    let Some(rc_exe) = locate_rc_exe() else {
        println!(
            "cargo:warning=rc.exe not found — building without embedded application icon; \
             install Visual Studio Build Tools with the Windows SDK, or use GitHub Actions release builds"
        );
        return;
    };

    let status = Command::new(&rc_exe)
        .current_dir(&assets)
        .args([
            "/nologo",
            &format!("/fo{}", res_out.display()),
            "trayvault.rc",
        ])
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", rc_exe.display()));

    if !status.success() {
        panic!("rc.exe failed compiling {}", rc_src.display());
    }

    println!("cargo:rustc-link-arg={}", res_out.display());
}

#[cfg(windows)]
fn locate_rc_exe() -> Option<PathBuf> {
    if let Ok(path) = env::var("PATH") {
        for dir in env::split_paths(&path) {
            let candidate = dir.join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    if let Ok(vc) = env::var("VCINSTALLDIR") {
        for sub in ["Hostx64\\x64", "Hostx86\\x86"] {
            let candidate = PathBuf::from(&vc).join("bin").join(sub).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    find_rc_via_vswhere().or_else(find_rc_in_program_files)
}

#[cfg(windows)]
fn find_rc_via_vswhere() -> Option<PathBuf> {
    let program_files = env::var_os("ProgramFiles(x86)")?;
    let vswhere = Path::new(&program_files)
        .join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe");
    if !vswhere.is_file() {
        return None;
    }

    let output = Command::new(&vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let install = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if install.is_empty() {
        return None;
    }

    let msvc_root = PathBuf::from(install).join("VC").join("Tools").join("MSVC");
    let Ok(entries) = std::fs::read_dir(&msvc_root) else {
        return None;
    };
    let mut versions: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    versions.sort();
    for ver in versions.into_iter().rev() {
        let candidate = ver.join("bin").join("Hostx64").join("x64").join("rc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(windows)]
fn find_rc_in_program_files() -> Option<PathBuf> {
    let roots = [
        env::var_os("ProgramFiles(x86)"),
        env::var_os("ProgramFiles"),
    ];
    for root in roots.into_iter().flatten() {
        let kits = PathBuf::from(&root)
            .join("Windows Kits")
            .join("10")
            .join("bin");
        let Ok(entries) = std::fs::read_dir(&kits) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        versions.sort();
        for ver in versions.into_iter().rev() {
            for arch in ["x64", "x86"] {
                let candidate = ver.join(arch).join("rc.exe");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
