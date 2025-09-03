// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::{Command, Stdio, Child};
use std::env;
use tauri::{Manager, WindowEvent};
use lazy_static::lazy_static;
use std::sync::{Arc, Mutex};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use rust_embed::RustEmbed;

// Embed the server binary as a resource
#[derive(RustEmbed)]
#[folder = "resources/"]
struct Resources;

// Глобальная статическая переменная для хранения дескриптора процесса сервера
lazy_static! {
    static ref SERVER_PROCESS: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
}

fn main() {
    let server_process_handle = Arc::clone(&SERVER_PROCESS);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(move |_app| {
            #[cfg(not(debug_assertions))] // Only execute in release mode
            {
                // Extract the embedded server binary
                let server_bin_path = extract_server_binary().expect("Failed to extract server binary");
                
                // Запускаем сервер
                let child_process = Command::new(&server_bin_path)
                    .stdout(Stdio::null()) // Hide stdout
                    .stderr(Stdio::null()) // Hide stderr
                    .spawn()
                    .expect("Failed to start zbx-np-server.exe");

                // Сохраняем дескриптор процесса
                *server_process_handle.lock().unwrap() = Some(child_process);
            }
            let main_window = _app.get_webview_window("main").unwrap();
            main_window.show().unwrap();
            Ok(())
        })
        .on_window_event(move |window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Проверяем, является ли это последним окном
                if window.app_handle().webview_windows().len() == 1 {
                    // При закрытии последнего окна завершаем процесс сервера
                    if let Some(mut child) = SERVER_PROCESS.lock().unwrap().take() {
                        let _ = child.kill(); // Пытаемся завершить процесс
                        let _ = child.wait(); // Ждем завершения
                        println!("Server process terminated.");
                        
                        // Clean up the extracted server binary
                        #[cfg(not(debug_assertions))]
                        {
                            let server_bin_path = env::current_exe()
                                .expect("Failed to get current executable path")
                                .parent()
                                .expect("Failed to get parent directory")
                                .join("zbx-np-server.exe");
                            if server_bin_path.exists() {
                                let _ = std::fs::remove_file(server_bin_path);
                            }
                        }
                        
                        window.app_handle().exit(0);
                    }
                }
                api.prevent_close();
            }
        });

    app.run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(not(debug_assertions))]
fn extract_server_binary() -> std::io::Result<std::path::PathBuf> {
    // Get the path where the Tauri app is running
    let current_dir = env::current_exe()?
        .parent()
        .expect("Failed to get parent directory")
        .to_path_buf();
    
    // Define the path for the extracted server binary
    let server_bin_path = current_dir.join("zbx-np-server.exe");
    
    // Check if the server binary already exists (in case of previous abnormal termination)
    if !server_bin_path.exists() {
        // Extract the embedded server binary
        if let Some(embedded_file) = Resources::get("zbx-np-server.exe") {
            let mut file = File::create(&server_bin_path)?;
            file.write_all(&embedded_file.data)?;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Embedded server binary not found"
            ));
        }
    }
    
    Ok(server_bin_path)
}