use std::env;
use std::fs;
use std::path::Path;

fn main() {
    tauri_build::build();
    
    // Embed the server binary as a resource
    #[cfg(not(debug_assertions))]
    {
        // Get the target directory
        let target_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let server_path = Path::new(&target_dir).parent().unwrap().join("target").join("release").join("zbx-np-server.exe");
        
        // Check if the server binary exists
        if server_path.exists() {
            // Copy the server binary to the tauri directory for embedding
            let tauri_resources = Path::new(&target_dir).join("resources");
            if !tauri_resources.exists() {
                fs::create_dir_all(&tauri_resources).unwrap();
            }
            
            let target_path = tauri_resources.join("zbx-np-server.exe");
            fs::copy(&server_path, &target_path).unwrap();
            
            // Tell Cargo to rerun this script if the server binary changes
            println!("cargo:rerun-if-changed={}", server_path.display());
        }
    }
}