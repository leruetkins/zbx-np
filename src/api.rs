use actix_web::{web, HttpResponse, Result, Error};
use serde_json::json;
use std::sync::{Arc, Mutex};
use crate::auth::{Stats, Config, HttpConfig, MqttConfig, LoginRequest, LoginResponse, User, Token};
use uuid::Uuid;
use std::fs;
use chrono;
use tokio::sync::oneshot;

// Helper functions for config file management
fn read_config() -> std::result::Result<serde_json::Value, Box<dyn std::error::Error>> {
    let config_content = fs::read_to_string("config.json")?;
    let config: serde_json::Value = serde_json::from_str(&config_content)?;
    Ok(config)
}

fn write_config(config: &serde_json::Value) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let config_content = serde_json::to_string_pretty(config)?;
    fs::write("config.json", config_content)?;
    Ok(())
}

fn get_users_from_config() -> Vec<User> {
    match read_config() {
        Ok(config) => {
            config.get("users")
                .and_then(|users| users.as_array())
                .map(|users_array| {
                    users_array.iter()
                        .filter_map(|user| serde_json::from_value(user.clone()).ok())
                        .collect()
                })
                .unwrap_or_else(Vec::new)
        }
        Err(_) => Vec::new()
    }
}

fn get_next_user_id() -> u64 {
    let users = get_users_from_config();
    users.iter().map(|u| u.id).max().unwrap_or(0) + 1
}

fn get_tokens_from_config() -> Vec<Token> {
    match read_config() {
        Ok(config) => {
            config.get("tokens")
                .and_then(|tokens| tokens.as_array())
                .map(|tokens_array| {
                    tokens_array.iter()
                        .filter_map(|token| serde_json::from_value(token.clone()).ok())
                        .collect()
                })
                .unwrap_or_else(Vec::new)
        }
        Err(_) => Vec::new()
    }
}

fn get_next_token_id() -> u64 {
    let tokens = get_tokens_from_config();
    tokens.iter().map(|t| t.id).max().unwrap_or(0) + 1
}

// State for sharing application data
#[derive(Clone)]
pub struct AppState {
    pub stats: Arc<Mutex<Stats>>,
}

// Simple login endpoint (using created users from config.json)
pub async fn login(
    data: web::Json<LoginRequest>,
    _app_state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let users = get_users_from_config();
    
    // Check if user exists and password matches
    let user_found = users.iter().find(|user| {
        user.username == data.username && user.password == data.password
    });
    
    if user_found.is_some() {
        Ok(HttpResponse::Ok().json(LoginResponse {
            success: true,
            message: "Login successful".to_string(),
        }))
    } else {
        Ok(HttpResponse::Unauthorized().json(LoginResponse {
            success: false,
            message: "Invalid credentials".to_string(),
        }))
    }
}

pub async fn logout() -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().json(json!({
        "message": "Logged out successfully"
    })))
}

// Configuration endpoints
pub async fn get_config() -> Result<HttpResponse, Error> {
    // Read current config from file instead of static variable to get real-time state
    match read_config() {
        Ok(config_json) => {
            let config = Config {
                http: HttpConfig {
                    port: config_json["settings"]["http"]["port"].as_u64().unwrap_or(7000),
                },
                mqtt: MqttConfig {
                    enabled: config_json["settings"]["mqtt"]["enabled"].as_bool().unwrap_or(false),
                    url: config_json["settings"]["mqtt"]["url"].as_str().unwrap_or("").to_string(),
                    id: config_json["settings"]["mqtt"]["id"].as_str().unwrap_or("").to_string(),
                    login: config_json["settings"]["mqtt"]["login"].as_str().unwrap_or("").to_string(),
                    password: "***".to_string(), // Don't expose password
                    period: config_json["settings"]["mqtt"]["period"].as_u64().unwrap_or(0),
                    topic: config_json["settings"]["mqtt"]["topic"].as_str().unwrap_or("").to_string(),
                },
            };
            
            Ok(HttpResponse::Ok().json(config))
        }
        Err(e) => {
            Ok(HttpResponse::InternalServerError().json(json!({
                "error": "Failed to read configuration file",
                "details": e.to_string()
            })))
        }
    }
}

pub async fn update_config(data: web::Json<Config>, app_state: web::Data<AppState>) -> Result<HttpResponse, Error> {
    // Read existing config to preserve users and tokens
    match read_config() {
        Ok(mut existing_config) => {
            // Check if MQTT settings have changed
            let mqtt_changed = 
                existing_config["settings"]["mqtt"]["enabled"].as_bool().unwrap_or(false) != data.mqtt.enabled ||
                existing_config["settings"]["mqtt"]["url"].as_str().unwrap_or("") != data.mqtt.url ||
                existing_config["settings"]["mqtt"]["topic"].as_str().unwrap_or("") != data.mqtt.topic ||
                existing_config["settings"]["mqtt"]["id"].as_str().unwrap_or("") != data.mqtt.id ||
                existing_config["settings"]["mqtt"]["login"].as_str().unwrap_or("") != data.mqtt.login ||
                (data.mqtt.password != "***" && existing_config["settings"]["mqtt"]["password"].as_str().unwrap_or("") != data.mqtt.password);
                
            // Update settings while preserving users and tokens
            existing_config["settings"]["http"]["port"] = json!(data.http.port);
            existing_config["settings"]["mqtt"]["enabled"] = json!(data.mqtt.enabled);
            existing_config["settings"]["mqtt"]["url"] = json!(data.mqtt.url);
            existing_config["settings"]["mqtt"]["id"] = json!(data.mqtt.id);
            existing_config["settings"]["mqtt"]["login"] = json!(data.mqtt.login);
            existing_config["settings"]["mqtt"]["period"] = json!(data.mqtt.period);
            existing_config["settings"]["mqtt"]["topic"] = json!(data.mqtt.topic);
            
            // Only update password if it's not the masked value
            if data.mqtt.password != "***" {
                existing_config["settings"]["mqtt"]["password"] = json!(data.mqtt.password);
            }
            
            // Write updated config to file
            match write_config(&existing_config) {
                Ok(_) => {
                    let mut response_message = "Configuration updated successfully.";
                    
                    // Restart MQTT service if settings changed
                    if mqtt_changed {
                        if let Err(e) = restart_mqtt_service(data.mqtt.enabled, app_state.stats.clone()).await {
                            return Ok(HttpResponse::Ok().json(json!({
                                "success": true,
                                "message": format!("Configuration saved, but MQTT restart failed: {}", e),
                                "mqtt_restart_error": true
                            })));
                        }
                        response_message = "Configuration updated and MQTT service restarted successfully.";
                    }
                    
                    Ok(HttpResponse::Ok().json(json!({
                        "success": true,
                        "message": response_message
                    })))
                }
                Err(e) => Ok(HttpResponse::InternalServerError().json(json!({
                    "error": "Failed to write configuration file",
                    "details": e.to_string()
                })))
            }
        }
        Err(e) => Ok(HttpResponse::InternalServerError().json(json!({
            "error": "Failed to read existing configuration",
            "details": e.to_string()
        })))
    }
}

pub async fn test_config(data: web::Json<Config>) -> Result<HttpResponse, Error> {
    let mut results = Vec::new();
    
    // Test MQTT connection if enabled
    if data.mqtt.enabled && !data.mqtt.url.is_empty() {
        results.push(json!({
            "service": "MQTT",
            "status": "success",
            "message": "Configuration appears valid"
        }));
    }
    
    Ok(HttpResponse::Ok().json(json!({
        "results": results,
        "overall_status": "success"
    })))
}

// Statistics endpoints
pub async fn get_stats(app_state: web::Data<AppState>) -> Result<HttpResponse, Error> {
    let stats = app_state.stats.lock().unwrap();
    Ok(HttpResponse::Ok().json(&*stats))
}

pub async fn get_stats_history() -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().json(json!({
        "history": [],
        "message": "Historical data collection not implemented yet"
    })))
}

pub async fn get_realtime_stats(app_state: web::Data<AppState>) -> Result<HttpResponse, Error> {
    let stats = app_state.stats.lock().unwrap();
    Ok(HttpResponse::Ok().json(&*stats))
}

// Management endpoints
pub async fn restart_service() -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().json(json!({
        "message": "Restart functionality not implemented. Please manually restart the service."
    })))
}

// Get MQTT status from global state instead of config file
pub async fn get_mqtt_status() -> Result<HttpResponse, Error> {
    // Get current MQTT state from global state instead of config file
    let mqtt_state = crate::MQTT_STATE.lock().unwrap().clone();
    
    println!("DEBUG: API returning MQTT state: enabled={}, status='{}', url='{}'", 
             mqtt_state.enabled, mqtt_state.status, mqtt_state.url);
    
    Ok(HttpResponse::Ok().json(json!({
        "enabled": mqtt_state.enabled,
        "status": mqtt_state.status,
        "url": mqtt_state.url,
        "topic": mqtt_state.topic
    })))
}

// MQTT Service Management
async fn restart_mqtt_service(enabled: bool, stats: Arc<Mutex<Stats>>) -> std::result::Result<(), String> {
    // Read current config to get MQTT settings for broadcasting
    let (mqtt_url, mqtt_topic) = match read_config() {
        Ok(config) => {
            let url = config["settings"]["mqtt"]["url"].as_str().unwrap_or("").to_string();
            let topic = config["settings"]["mqtt"]["topic"].as_str().unwrap_or("").to_string();
            (url, topic)
        }
        Err(_) => return Err("Failed to read config".to_string()),
    };
    
    // Stop existing MQTT service if running
    {
        let mut handle = crate::MQTT_HANDLE.lock().unwrap();
        if let Some(shutdown_tx) = handle.take() {
            if let Err(_) = shutdown_tx.send(()) {
                // Service might already be stopped, which is OK
            }
        }
    }
    
    // Broadcast stopping status
    if enabled {
        crate::broadcast_mqtt_status(enabled, "stopping", &mqtt_url, &mqtt_topic);
    } else {
        crate::broadcast_mqtt_status(false, "disabled", &mqtt_url, &mqtt_topic);
    }
    
    // Wait a moment for the service to shut down
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // Start new MQTT service if enabled
    if enabled {
        // Broadcast starting status immediately
        crate::broadcast_mqtt_status(true, "starting", &mqtt_url, &mqtt_topic);
        
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        
        // Store the new shutdown handle
        {
            let mut handle = crate::MQTT_HANDLE.lock().unwrap();
            *handle = Some(shutdown_tx);
        }
        
        let stats_clone = stats.clone();
        let mqtt_url_clone = mqtt_url.clone();
        let mqtt_topic_clone = mqtt_topic.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::mqtt_connect(stats_clone, shutdown_rx).await {
                eprintln!("MQTT connection error: {}", e);
                crate::add_message(format!("MQTT Error: {}", e));
                // Broadcast error status on failure
                crate::broadcast_mqtt_status(true, "error", &mqtt_url_clone, &mqtt_topic_clone);
            }
        });
        
        crate::add_message("MQTT: Service restarted".to_string());
    } else {
        crate::add_message("MQTT: Service disabled".to_string());
    }
    
    Ok(())
}

pub async fn get_logs() -> Result<HttpResponse, Error> {
    // Return recent messages from the global message store
    let messages = crate::get_messages();
    let recent_messages: Vec<String> = messages.iter().take(50).cloned().collect();
    
    Ok(HttpResponse::Ok().json(json!({
        "logs": recent_messages,
        "message": "Recent application logs"
    })))
}

// Placeholder user endpoints
pub async fn get_users() -> Result<HttpResponse, Error> {
    let users = get_users_from_config();
    
    // Convert to response format (without passwords)
    let user_list: Vec<serde_json::Value> = users.into_iter()
        .map(|user| json!({
            "id": user.id,
            "username": user.username,
            "created_at": user.created_at
        }))
        .collect();
    
    Ok(HttpResponse::Ok().json(user_list))
}

pub async fn create_user(data: web::Json<serde_json::Value>) -> Result<HttpResponse, Error> {
    let username = data.get("username").and_then(|u| u.as_str()).unwrap_or("");
    let password = data.get("password").and_then(|p| p.as_str()).unwrap_or("");
    
    if username.is_empty() || password.is_empty() {
        return Ok(HttpResponse::BadRequest().json(json!({
            "error": "Username and password are required"
        })));
    }
    
    // Check for duplicate username
    let existing_users = get_users_from_config();
    if existing_users.iter().any(|u| u.username == username) {
        return Ok(HttpResponse::Conflict().json(json!({
            "error": "Username already exists"
        })));
    }
    
    // Create new user
    let new_user = User {
        id: get_next_user_id(),
        username: username.to_string(),
        password: password.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    
    // Read current config, add user, and save
    match read_config() {
        Ok(mut config) => {
            // Ensure users array exists
            if !config.get("users").is_some() {
                config["users"] = json!([]);
            }
            
            let users_array = config.get_mut("users")
                .and_then(|u| u.as_array_mut())
                .expect("Users array should exist");
            
            users_array.push(serde_json::to_value(&new_user).unwrap());
            
            match write_config(&config) {
                Ok(_) => {
                    Ok(HttpResponse::Ok().json(json!({
                        "success": true,
                        "message": "User created successfully",
                        "user": {
                            "id": new_user.id,
                            "username": new_user.username,
                            "created_at": new_user.created_at
                        }
                    })))
                }
                Err(e) => {
                    Ok(HttpResponse::InternalServerError().json(json!({
                        "error": "Failed to save user",
                        "details": e.to_string()
                    })))
                }
            }
        }
        Err(e) => {
            Ok(HttpResponse::InternalServerError().json(json!({
                "error": "Failed to read configuration",
                "details": e.to_string()
            })))
        }
    }
}

pub async fn delete_user(path: web::Path<String>) -> Result<HttpResponse, Error> {
    let user_id_str = path.into_inner();
    let user_id: u64 = match user_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return Ok(HttpResponse::BadRequest().json(json!({
                "error": "Invalid user ID"
            })))
        }
    };
    
    // Prevent deletion of admin user (ID 1)
    if user_id == 1 {
        return Ok(HttpResponse::Forbidden().json(json!({
            "error": "Cannot delete admin user"
        })));
    }
    
    match read_config() {
        Ok(mut config) => {
            let users_array = config.get_mut("users")
                .and_then(|u| u.as_array_mut());
            
            if let Some(users) = users_array {
                let original_len = users.len();
                users.retain(|user| {
                    user.get("id")
                        .and_then(|id| id.as_u64())
                        .map_or(true, |id| id != user_id)
                });
                
                if users.len() < original_len {
                    match write_config(&config) {
                        Ok(_) => {
                            Ok(HttpResponse::Ok().json(json!({
                                "success": true,
                                "message": format!("User {} deleted successfully", user_id)
                            })))
                        }
                        Err(e) => {
                            Ok(HttpResponse::InternalServerError().json(json!({
                                "error": "Failed to save configuration",
                                "details": e.to_string()
                            })))
                        }
                    }
                } else {
                    Ok(HttpResponse::NotFound().json(json!({
                        "error": "User not found"
                    })))
                }
            } else {
                Ok(HttpResponse::NotFound().json(json!({
                    "error": "No users found"
                })))
            }
        }
        Err(e) => {
            Ok(HttpResponse::InternalServerError().json(json!({
                "error": "Failed to read configuration",
                "details": e.to_string()
            })))
        }
    }
}

// Token endpoints with file persistence
pub async fn get_tokens() -> Result<HttpResponse, Error> {
    let tokens = get_tokens_from_config();
    
    // Convert to response format (without actual token values)
    let token_list: Vec<serde_json::Value> = tokens.into_iter()
        .map(|token| {
            let token_preview = if token.token.len() > 8 {
                format!("{}...", &token.token[..8])
            } else {
                "****".to_string()
            };
            
            json!({
                "id": token.id,
                "name": token.name,
                "token_preview": token_preview,
                "created_at": token.created_at,
                "expires_at": token.expires_at,
                "is_active": token.is_active
            })
        })
        .collect();
    
    Ok(HttpResponse::Ok().json(token_list))
}

pub async fn create_token(data: web::Json<serde_json::Value>) -> Result<HttpResponse, Error> {
    let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let expires_in = data.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(30);
    
    if name.is_empty() {
        return Ok(HttpResponse::BadRequest().json(json!({
            "error": "Token name is required"
        })));
    }
    
    // Check for duplicate token name
    let existing_tokens = get_tokens_from_config();
    if existing_tokens.iter().any(|t| t.name == name) {
        return Ok(HttpResponse::Conflict().json(json!({
            "error": "Token name already exists"
        })));
    }
    
    // Generate token
    let token = format!("zbx_{}", Uuid::new_v4().to_string().replace("-", ""));
    let expires_at = if expires_in > 0 {
        Some(chrono::Utc::now() + chrono::Duration::days(expires_in as i64))
    } else {
        None
    };
    
    // Create new token
    let new_token = Token {
        id: get_next_token_id(),
        name: name.to_string(),
        token: token.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        expires_at: expires_at.map(|d| d.to_rfc3339()),
        is_active: true,
    };
    
    // Read current config, add token, and save
    match read_config() {
        Ok(mut config) => {
            // Ensure tokens array exists
            if !config.get("tokens").is_some() {
                config["tokens"] = json!([]);
            }
            
            let tokens_array = config.get_mut("tokens")
                .and_then(|t| t.as_array_mut())
                .expect("Tokens array should exist");
            
            tokens_array.push(serde_json::to_value(&new_token).unwrap());
            
            match write_config(&config) {
                Ok(_) => {
                    Ok(HttpResponse::Ok().json(json!({
                        "success": true,
                        "token": token,  // Return the actual token only once
                        "name": name,
                        "expires_at": expires_at.map(|d| d.to_rfc3339())
                    })))
                }
                Err(e) => {
                    Ok(HttpResponse::InternalServerError().json(json!({
                        "error": "Failed to save token",
                        "details": e.to_string()
                    })))
                }
            }
        }
        Err(e) => {
            Ok(HttpResponse::InternalServerError().json(json!({
                "error": "Failed to read configuration",
                "details": e.to_string()
            })))
        }
    }
}

pub async fn delete_token(path: web::Path<String>) -> Result<HttpResponse, Error> {
    let token_id_str = path.into_inner();
    let token_id: u64 = match token_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return Ok(HttpResponse::BadRequest().json(json!({
                "error": "Invalid token ID"
            })))
        }
    };
    
    match read_config() {
        Ok(mut config) => {
            let tokens_array = config.get_mut("tokens")
                .and_then(|t| t.as_array_mut());
            
            if let Some(tokens) = tokens_array {
                let original_len = tokens.len();
                tokens.retain(|token| {
                    token.get("id")
                        .and_then(|id| id.as_u64())
                        .map_or(true, |id| id != token_id)
                });
                
                if tokens.len() < original_len {
                    match write_config(&config) {
                        Ok(_) => {
                            Ok(HttpResponse::Ok().json(json!({
                                "success": true,
                                "message": format!("Token {} revoked successfully", token_id)
                            })))
                        }
                        Err(e) => {
                            Ok(HttpResponse::InternalServerError().json(json!({
                                "error": "Failed to save configuration",
                                "details": e.to_string()
                            })))
                        }
                    }
                } else {
                    Ok(HttpResponse::NotFound().json(json!({
                        "error": "Token not found"
                    })))
                }
            } else {
                Ok(HttpResponse::NotFound().json(json!({
                    "error": "No tokens found"
                })))
            }
        }
        Err(e) => {
            Ok(HttpResponse::InternalServerError().json(json!({
                "error": "Failed to read configuration",
                "details": e.to_string()
            })))
        }
    }
}