@echo off
REM Batch script to build zbx-np server and then Tauri application with embedded server

echo Building zbx-np server...
cargo build --bin zbx-np-server --release
if %errorlevel% neq 0 (
    echo Error: Failed to build zbx-np server
    exit /b %errorlevel%
)

echo Copying server binary to Tauri resources...
copy "target\release\zbx-np-server.exe" "src-tauri\resources\"
if %errorlevel% neq 0 (
    echo Error: Failed to copy server binary to Tauri resources
    exit /b %errorlevel%
)

echo Building Tauri application...
cd src-tauri
cargo build --release
if %errorlevel% neq 0 (
    echo Error: Failed to build Tauri application
    exit /b %errorlevel%
)

echo Build completed successfully!
echo Tauri executable is located at: src-tauri\target\release\zbx-np-tauri.exe