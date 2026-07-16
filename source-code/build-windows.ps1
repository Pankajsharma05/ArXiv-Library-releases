# Build ArXivLibrary for Windows (.msi and .exe/NSIS installer).
#
# PREREQUISITES (one-time):
#   1. Microsoft C++ Build Tools (the "Desktop development with C++" workload):
#        https://visualstudio.microsoft.com/visual-cpp-build-tools/
#   2. Rust toolchain (MSVC):  https://rustup.rs   (choose the default MSVC host)
#   3. WebView2 runtime — preinstalled on Windows 10/11. If missing:
#        https://developer.microsoft.com/microsoft-edge/webview2/
#   4. Tauri CLI:
#        cargo install tauri-cli --version "^2"
#
# Then, from the project root in PowerShell:
#   .\build-windows.ps1
#
# If script execution is blocked, run once:
#   Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned

$ErrorActionPreference = "Stop"
Set-Location -Path $PSScriptRoot

Write-Host "==> Building ArXivLibrary for Windows..." -ForegroundColor Cyan
cargo tauri build

Write-Host ""
Write-Host "==> Done. Installers are in:" -ForegroundColor Green
Write-Host "    src-tauri\target\release\bundle\msi\    (.msi  - Windows Installer)"
Write-Host "    src-tauri\target\release\bundle\nsis\   (.exe  - NSIS setup)"
Write-Host ""
Write-Host "The raw executable (no installer) is at:"
Write-Host "    src-tauri\target\release\ArXivLibrary.exe"
