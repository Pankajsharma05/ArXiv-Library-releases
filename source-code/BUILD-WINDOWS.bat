@echo off
REM ===================================================================
REM  ArXivLibrary - One-click Windows build
REM
REM  Just double-click this file, or run it from a terminal:
REM      BUILD-WINDOWS.bat
REM
REM  It checks for everything needed, installs what's missing, and
REM  builds the app. No Node.js required.
REM ===================================================================

setlocal
cd /d "%~dp0"

echo.
echo ===========================================
echo   ArXivLibrary - Windows Build
echo ===========================================
echo.

REM ---------- 1. Check for Rust ----------
where cargo >nul 2>&1
if %errorlevel% equ 0 goto :rust_ok

echo [1/3] Rust not found. Installing Rust...
echo.
echo       Downloading rustup-init.exe...
powershell -NoProfile -Command "Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile '%TEMP%\rustup-init.exe'"
if %errorlevel% neq 0 goto :rust_download_failed

echo       Running Rust installer. This may take a few minutes...
"%TEMP%\rustup-init.exe" -y --default-toolchain stable --profile default
if %errorlevel% neq 0 goto :rust_install_failed
echo       Rust installed.
goto :rust_done

:rust_download_failed
echo.
echo ERROR: Could not download the Rust installer. Check your internet connection,
echo        or install manually from https://rustup.rs
pause
exit /b 1

:rust_install_failed
echo ERROR: Rust installation failed.
pause
exit /b 1

:rust_ok
echo [1/3] Rust found.

:rust_done
echo.

REM Make sure cargo is on PATH, whether just installed or installed previously.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

REM ---------- 2. Check for MSVC C++ build tools ----------
REM Rust on Windows needs the MSVC linker. Locate vswhere outside any if-block:
REM %ProgramFiles(x86)% contains literal parentheses which break batch parsing
REM if expanded inside a parenthesised block, so resolve it up front.
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"

where link.exe >nul 2>&1
if %errorlevel% equ 0 goto :msvc_ok

if not exist "%VSWHERE%" goto :msvc_missing
"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath >nul 2>&1
if %errorlevel% equ 0 goto :msvc_ok

:msvc_missing
echo [2/3] MSVC C++ build tools NOT found.
echo.
echo       Rust needs the Microsoft C++ build tools to link on Windows.
echo       This is a one-time install and cannot be automated here.
echo.
echo       Please install "Build Tools for Visual Studio":
echo         https://visualstudio.microsoft.com/visual-cpp-build-tools/
echo.
echo       In the installer, tick the workload:
echo         "Desktop development with C++"
echo.
echo       Then re-run this script.
echo.
set /p OPENIT="Open the download page now? (y/n): "
if /i "%OPENIT%"=="y" start https://visualstudio.microsoft.com/visual-cpp-build-tools/
pause
exit /b 1

:msvc_ok
echo [2/3] MSVC C++ build tools found.
echo.

REM ---------- 3. Check for Tauri CLI ----------
cargo tauri --version >nul 2>&1
if %errorlevel% equ 0 goto :tauri_ok

echo [3/3] Tauri CLI not found. Installing it now; this takes several minutes...
cargo install tauri-cli --version "^2" --locked
if %errorlevel% neq 0 goto :tauri_failed
echo       Tauri CLI installed.
goto :tauri_done

:tauri_failed
echo ERROR: Failed to install the Tauri CLI.
pause
exit /b 1

:tauri_ok
echo [3/3] Tauri CLI found.

:tauri_done
echo.

REM ---------- WebView2 runtime check (needed to RUN, not to build) ----------
REM Preinstalled on Windows 11 and most Windows 10. Warn if absent.
reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" >nul 2>&1
if %errorlevel% equ 0 goto :webview_ok
reg query "HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" >nul 2>&1
if %errorlevel% equ 0 goto :webview_ok

echo NOTE: WebView2 runtime was not detected.
echo       The app will still build, but needs WebView2 to run.
echo       It ships with Windows 11 and recent Windows 10. If the app
echo       fails to launch, install it from:
echo         https://developer.microsoft.com/microsoft-edge/webview2/
echo.

:webview_ok

REM ---------- Build ----------
echo ===========================================
echo   Building... (first build is slow: 5-15 min)
echo ===========================================
echo.

cargo tauri build
if %errorlevel% equ 0 goto :build_ok

echo.
echo ===========================================
echo   BUILD FAILED
echo ===========================================
echo   Scroll up for the error message.
echo.
echo   Common causes:
echo     - MSVC "Desktop development with C++" workload not installed
echo     - No internet connection; dependencies must be downloaded
echo.
pause
exit /b 1

:build_ok

echo.
echo ===========================================
echo   BUILD SUCCESSFUL
echo ===========================================
echo.
echo   Installers:
echo     src-tauri\target\release\bundle\msi\    (.msi)
echo     src-tauri\target\release\bundle\nsis\   (.exe setup)
echo.
echo   Standalone executable (no installer):
echo     src-tauri\target\release\ArXivLibrary.exe
echo.

REM Offer to open the output folder
set /p OPENOUT="Open the output folder? (y/n): "
if /i not "%OPENOUT%"=="y" goto :done
if exist "src-tauri\target\release\bundle" goto :open_bundle
start "" "src-tauri\target\release"
goto :done

:open_bundle
start "" "src-tauri\target\release\bundle"

:done
pause
endlocal
