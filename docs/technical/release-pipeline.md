# Release Pipeline

Modules: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `build.rs`, `assets/trayvault.rc`, `installer/trayvault.wxs`.

## CI (`ci.yml`)

Runs on push/PR to `main` or `master` on `windows-latest`:

1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo build`
4. `cargo test`

Rust toolchain is pinned to **1.85** (MSRV in `Cargo.toml`).

## Release (`release.yml`)

Triggers on tags matching `v*` (e.g. `v0.1.0`).

1. Parses semver from the tag (`v0.1.0` → `0.1.0`) and rewrites `Cargo.toml` `version` for the build.
2. `cargo build --release --target x86_64-pc-windows-msvc`
3. Stages `dist/trayvault.exe`, `icon.ico`, `LICENSE`, `README.md`
4. Builds MSI with **WiX Toolset 3.11** (`candle` + `light` + `WixUIExtension`)
5. Zips exe + icon + LICENSE + README → `trayvault-windows-x86_64.zip`
6. Uploads to the GitHub Release:
   - `trayvault.exe` (standalone)
   - `trayvault-windows-x86_64.zip`
   - `trayvault-windows-x86_64.msi`

Create a release by pushing a tag:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

## Executable icon (`build.rs`)

`assets/trayvault.rc` references `icon.ico`. `build.rs` runs `rc.exe` (Windows SDK) and links `trayvault.res` into the binary so Explorer shows the app icon. Tray code still uses `include_bytes!("../../assets/icon.ico")` for `LoadImageW` from memory.

## MSI installer (`installer/trayvault.wxs`)

- **Scope:** per-user (`InstallScope="perUser"`) → `%LOCALAPPDATA%\TrayVault\trayvault.exe`
- **UI:** `WixUI_FeatureTree` — on **Custom** setup, the user can toggle:
  - **Start TrayVault when Windows starts** — writes `HKCU\...\Run\TrayVault` = `"<exe>" --minimized` (same as [`autostart.rs`](../../src/win32/autostart.rs))
  - **Start Menu shortcut**
- Typical install includes Level 1 features (app + autostart + shortcut) by default; Custom allows turning autostart off before install.
- Uninstall removes Run-key value and shortcuts when those components were installed.

After MSI install, in-app **Settings → Start with Windows** remains the source of truth for toggling autostart on later runs.

## Local MSI build (optional)

```powershell
cargo build --release
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\trayvault.exe dist\
Copy-Item assets\icon.ico dist\
$env:Path = "${env:ProgramFiles(x86)}\WiX Toolset v3.11\bin;$env:Path"
candle -nologo -arch x64 -dProductVersion=0.1.0 -dSourceDir=(Resolve-Path dist) -dProjectDir=(Get-Location) installer\trayvault.wxs -out installer\trayvault.wixobj
light -nologo -ext WixUIExtension installer\trayvault.wixobj -out dist\trayvault-windows-x86_64.msi
```
