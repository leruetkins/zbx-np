@echo off
REM Comprehensive batch script to build zbx-np server and then Tauri application with embedded server
REM Also creates installable bundles (MSI and NSIS)

echo ================================
echo Building zbx-np server...
echo ================================
cargo build --bin zbx-np-server --release
if %errorlevel% neq 0 (
    echo Error: Failed to build zbx-np server
    exit /b %errorlevel%
)

echo ================================
echo Copying server binary to Tauri resources...
echo ================================
copy "target\release\zbx-np-server.exe" "src-tauri\resources\"
if %errorlevel% neq 0 (
    echo Error: Failed to copy server binary to Tauri resources
    exit /b %errorlevel%
)

echo ================================
echo Building and bundling Tauri application...
echo ================================
tauri build
if %errorlevel% neq 0 (
    echo Error: Failed to build Tauri application
    exit /b %errorlevel%
)

echo ================================
echo Build and bundling completed successfully!
echo ================================
echo Executables and installers are located at:
echo   - Executable: src-tauri\target\release\zbx-np-tauri.exe
echo   - MSI Installer: src-tauri\target\release\bundle\msi\zbx-np-tauri_0.1.0_x64_en-US.msi
echo   - NSIS Installer: src-tauri\target\release\bundle\nsis\zbx-np-tauri_0.1.0_x64-setup.exe