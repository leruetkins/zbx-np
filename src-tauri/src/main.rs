// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::{Command, Stdio, Child};
use std::env;
use tauri::{Manager, WindowEvent}; // Изменено: RunEvent на WindowEvent
use lazy_static::lazy_static;
use std::sync::{Arc, Mutex}; // Добавлено Arc

// Глобальная статическая переменная для хранения дескриптора процесса сервера
lazy_static! {
    static ref SERVER_PROCESS: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
}

fn main() {
    let server_process_handle = Arc::clone(&SERVER_PROCESS); // Клонируем Arc для захвата в замыкание

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build()) // <-- Добавляем плагин здесь
        .setup(move |app| { // Используем move, чтобы захватить server_process_handle
            #[cfg(not(debug_assertions))] // Only execute in release mode
            {
                // Запуск сервера, предполагая, что он находится рядом с исполняемым файлом Tauri
                let server_bin_path = env::current_exe()
                    .expect("Failed to get current executable path")
                    .parent()
                    .expect("Failed to get parent directory")
                    .join("zbx-np-server.exe");

                // Запускаем сервер
                let child_process = Command::new(&server_bin_path)
                    .stdout(Stdio::null()) // Hide stdout
                    .stderr(Stdio::null()) // Hide stderr
                    .spawn()
                    .expect("Failed to start zbx-np-server.exe");

                // Сохраняем дескриптор процесса
                *server_process_handle.lock().unwrap() = Some(child_process);
            }
            Ok(())
        })
        .on_window_event(move |window, event| { // Используем on_window_event и move
            if let WindowEvent::CloseRequested { api, .. } = event { // Проверяем событие закрытия
                // Проверяем, является ли это последним окном
                if window.app_handle().webview_windows().len() == 1 {
                    // При закрытии последнего окна завершаем процесс сервера
                    if let Some(mut child) = SERVER_PROCESS.lock().unwrap().take() {
                        let _ = child.kill(); // Пытаемся завершить процесс
                        let _ = child.wait(); // Ждем завершения
                        println!("Server process terminated.");
                        window.app_handle().exit(0); // <-- Изменено здесь
                    }
                }
                api.prevent_close();
                // window.close(); // Эта строка теперь не нужна
            }
        });

    app.run(tauri::generate_context!())
        .expect("error while running tauri application");
}
