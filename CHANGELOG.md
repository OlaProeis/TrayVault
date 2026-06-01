# Changelog

All notable changes to TrayVault are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-06-01

### Added

- Clipboard history for text, rich text, and images with deduplication and configurable cap
- Searchable history with filter chips, pin/unpin, and context menu actions
- Borderless main window with custom title bar, themes (light / dark / system), and sticky search header
- Bilinear-filtered image thumbnails and full-screen preview modal
- System tray integration with show/hide and quit
- Global hotkey toggle (default `Alt+V`, configurable)
- Optional Windows autostart via Run key and `--minimized` startup flag
- Settings panel for hotkey, history limits, capture pause, taskbar visibility, and themes
- Local storage under `%LOCALAPPDATA%\TrayVault\` (metadata, image blobs, config, log)
- GitHub Actions CI (`fmt`, `clippy`, `build`, `test`) and release workflow (`.exe`, zip, MSI)
- MSI installer with optional **Start TrayVault when Windows starts** feature (Run key, tray-only launch)
- Roboto font attribution (`NOTICES.md`, `assets/Roboto-LICENSE.txt`)

### Fixed

- Search no longer surfaces image entries when typing — screenshot filenames and source-app metadata had caused false matches on common letters

### Distribution

- **Standalone:** `trayvault.exe` from GitHub Releases
- **Portable zip:** `trayvault-windows-x86_64.zip` (exe + icon + README + LICENSE + font notices)
- **Installer:** `trayvault-windows-x86_64.msi` (per-user install under `%LOCALAPPDATA%\TrayVault`, optional autostart and Start Menu shortcut)

[0.1.0]: https://github.com/OlaProeis/TrayVault/releases/tag/v0.1.0
