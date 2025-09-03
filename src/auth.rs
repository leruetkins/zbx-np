use serde::{Deserialize, Serialize};

// Simple structures for authentication
#[derive(Debug, Serialize)]
pub struct Stats {
    pub total_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub mqtt_messages: i64,
    pub zabbix_sends: i64,
    pub connected_clients: i32,
    pub uptime: String,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            mqtt_messages: 0,
            zabbix_sends: 0,
            connected_clients: 0,
            uptime: "0s".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub http: HttpConfig,
    pub mqtt: MqttConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpConfig {
    pub port: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MqttConfig {
    pub enabled: bool,
    pub url: String,
    pub id: String,
    pub login: String,
    pub password: String,
    pub period: u64,
    pub topic: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub password: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Token {
    pub id: u64,
    pub name: String,
    pub token: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub message: String,
}