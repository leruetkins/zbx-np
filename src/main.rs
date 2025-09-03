// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod api;

use actix_web::error::ErrorUnauthorized;
use actix_web::HttpRequest;
use actix_web::{ dev::ServiceRequest, Error };
use actix_web::{ get, post, web, App, HttpResponse, HttpServer, Result };
use actix_web_httpauth::{ extractors::basic::BasicAuth, middleware::HttpAuthentication };
use actix_cors::Cors;
use chrono::Local;
use once_cell::sync::Lazy;
use rumqttc::{AsyncClient, MqttOptions, QoS, Event, Packet, TlsConfiguration, Transport};
use serde::{ Deserialize, Serialize };
use serde_json::json;
use serde_json::Number;
use serde_json::Value;
use std::fs;
use std::io::{ Read, Write };
use std::net::{ SocketAddr, TcpStream };
use std::time::{Duration, Instant};
use std::sync::{ Arc, Mutex };
use std::thread;
use tokio::time;
use websocket::sync::Server;
use websocket::{ Message, OwnedMessage };
use lazy_static::lazy_static;
use websocket::sender::Writer;
use std::sync::{ MutexGuard };
use crossbeam_channel::{ unbounded, Sender, Receiver };

// Import auth and API modules
use auth::{ Stats };
use api::{ AppState };


static mut GLOBAL_MESSAGES: Vec<String> = Vec::new();
const HTML: &'static str = include_str!("admin.html");

const ZABBIX_MAX_LEN: usize = 300;
const ZABBIX_TIMEOUT: u64 = 1000;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

// Function to create default config if it doesn't exist
fn create_default_config() -> std::io::Result<()> {
    let default_config = json!({
        "settings": {
            "http": {
                "port": 7000,
                "login": "admin",
                "password": "admin"
            },
            "mqtt": {
                "enabled": false,
                "url": "",
                "id": "",
                "login": "",
                "password": "",
                "period": 0,
                "topic": ""
            }
        },
        "users": [
            {
                "id": 1,
                "username": "admin",
                "password": "admin",
                "created_at": chrono::Local::now().to_rfc3339()
            }
        ]
    });
    
    let config_content = serde_json::to_string_pretty(&default_config)?;
    fs::write("config.json", config_content)?;
    println!("Created default config.json with admin user (username: admin, password: admin)");
    Ok(())
}

// Function to load or create config
fn load_or_create_config() -> serde_json::Value {
    match fs::read_to_string("config.json") {
        Ok(config_content) => {
            match serde_json::from_str(&config_content) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Invalid JSON format in config.json: {}", e);
                    eprintln!("Please fix the JSON syntax or delete the file to create a new default config.");
                    std::process::exit(1);
                }
            }
        }
        Err(_) => {
            println!("Config file not found, creating default config.json...");
            if let Err(e) = create_default_config() {
                eprintln!("Failed to create default config: {}", e);
                std::process::exit(1);
            }
            // Load the newly created config
            let config_content = fs::read_to_string("config.json").expect("Failed to read newly created config");
            serde_json::from_str(&config_content).expect("Failed to parse newly created config")
        }
    }
}

static CONFIG_JSON: Lazy<serde_json::Value> = Lazy::new(|| {
    load_or_create_config()
});

lazy_static! {
    static ref SENDERS: Arc<Mutex<Vec<Arc<Mutex<Writer<TcpStream>>>>>> = Arc::new(
        Mutex::new(Vec::new())
    );
}

lazy_static! {
    static ref MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());
}

// Global MQTT service handle
lazy_static! {
    static ref MQTT_HANDLE: Arc<Mutex<Option<Sender<()>>>> = Arc::new(Mutex::new(None));
}

// Global MQTT state tracking with comprehensive information
#[derive(Clone, Debug)]
struct MqttState {
    enabled: bool,
    status: String,
    url: String,
    topic: String,
    last_updated: chrono::DateTime<chrono::Local>,
}

impl Default for MqttState {
    fn default() -> Self {
        Self {
            enabled: false,
            status: "unknown".to_string(),
            url: String::new(),
            topic: String::new(),
            last_updated: chrono::Local::now(),
        }
    }
}

lazy_static! {
    static ref MQTT_STATE: Arc<Mutex<MqttState>> = Arc::new(Mutex::new(MqttState::default()));
}

fn add_message(message: String) {
    let mut messages = MESSAGES.lock().unwrap();
    messages.insert(0, message.clone());
    // Immediately broadcast the message to all connected WebSocket clients
    drop(messages); // Release the lock before sending
    send_message(&message);
}

fn get_messages() -> MutexGuard<'static, Vec<String>> {
    MESSAGES.lock().unwrap()
}

fn send_message(message: &str) {
    let senders = SENDERS.lock().unwrap();
    
    // Clean message for WebSocket safety - allow UTF-8 text including Russian/Cyrillic
    // but filter out problematic control characters that could break WebSocket connections
    let clean_message = message.chars()
        .filter(|c| {
            // Allow printable characters, whitespace, and most Unicode characters
            // but exclude control characters (except newlines, tabs, carriage returns)
            !c.is_control() || matches!(c, '\n' | '\r' | '\t')
        })
        .take(500) // Limit to 500 characters
        .collect::<String>();
    
    if clean_message.trim().is_empty() {
        return; // Don't send empty messages
    }
    
    for sender in &*senders {
        let message = Message::text(&clean_message);
        if let Err(err) = sender.lock().unwrap().send_message(&message) {
            eprintln!("Error sending message: {:?}", err);
        }
    }
}

// Send structured WebSocket message for different types of updates
fn send_websocket_message(msg_type: &str, data: serde_json::Value) {
    let message = json!({
        "type": msg_type,
        "data": data,
        "timestamp": chrono::Local::now().to_rfc3339()
    });
    
    if let Ok(message_str) = serde_json::to_string(&message) {
        send_message(&message_str);
    }
}

// Broadcast MQTT status change via WebSocket
fn broadcast_mqtt_status(enabled: bool, status: &str, url: &str, topic: &str) {
    // Update global state with comprehensive information
    {
        let mut mqtt_state = MQTT_STATE.lock().unwrap();
        mqtt_state.enabled = enabled;
        mqtt_state.status = status.to_string();
        mqtt_state.url = url.to_string();
        mqtt_state.topic = topic.to_string();
        mqtt_state.last_updated = chrono::Local::now();
        println!("DEBUG: Updating MQTT state: enabled={}, status='{}', url='{}'", enabled, status, url);
    }
    
    let status_data = json!({
        "enabled": enabled,
        "status": status,
        "url": url,
        "topic": topic
    });
    
    send_websocket_message("mqtt_status", status_data);
}

// Send current MQTT status to a specific WebSocket client
fn send_current_mqtt_status_to_client(client_sender: &Arc<Mutex<Writer<TcpStream>>>) {
    let mqtt_state = MQTT_STATE.lock().unwrap().clone();
    
    println!("DEBUG: Sending current MQTT state to new client: enabled={}, status='{}', url='{}'", 
             mqtt_state.enabled, mqtt_state.status, mqtt_state.url);
    
    let status_message = json!({
        "type": "mqtt_status",
        "data": {
            "enabled": mqtt_state.enabled,
            "status": mqtt_state.status,
            "url": mqtt_state.url,
            "topic": mqtt_state.topic
        },
        "timestamp": chrono::Local::now().to_rfc3339()
    });
    
    if let Ok(message_str) = serde_json::to_string(&status_message) {
        let message = Message::text(message_str);
        if let Err(err) = client_sender.lock().unwrap().send_message(&message) {
            eprintln!("Error sending MQTT status to new client: {:?}", err);
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Data {
    zabbix_server: String,
    item_host_name: String,
    item: Vec<Item>,
}

#[derive(Deserialize, Serialize)]
struct Item {
    key: String,
    value: Number,
}

#[derive(Deserialize)]
struct UrlQuery {
    data: String,
}

pub struct ZabbixSender {
    zabbix_server_addr: SocketAddr,
    zabbix_item_host_name: String,
    zabbix_item_list: Vec<(String, String)>,
    zabbix_packet: [u8; ZABBIX_MAX_LEN],
}

impl ZabbixSender {
    pub fn new(zabbix_server_addr: SocketAddr, zabbix_item_host_name: String) -> Self {
        ZabbixSender {
            zabbix_server_addr,
            zabbix_item_host_name,
            zabbix_item_list: Vec::new(),
            zabbix_packet: [0; ZABBIX_MAX_LEN],
        }
    }

    pub fn send(&mut self) -> Result<String, std::io::Error> {
        let packet_len = self.create_zabbix_packet()?;
        let mut stream = TcpStream::connect(self.zabbix_server_addr)?;
        stream.write_all(&self.zabbix_packet[..packet_len])?;
        let mut buf = [0; 1024];
        let mut bytes_read = 0;
        for _ in 0..ZABBIX_TIMEOUT / 10 {
            if let Ok(n) = stream.read(&mut buf) {
                bytes_read = n;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if bytes_read > 0 {
            let show_result = std::str::from_utf8(&buf[..bytes_read]).unwrap();
            return Ok(show_result.to_string()); // Return the show_result value
        } else {
            println!("No result");
        }
        Err(std::io::Error::new(std::io::ErrorKind::Other, "Send operation failed"))
    }

    pub fn clear_item(&mut self) {
        self.zabbix_item_list.clear();
    }

    pub fn add_item(&mut self, key: String, value: String) {
        self.zabbix_item_list.push((key, value));
    }

    fn create_zabbix_packet(&mut self) -> Result<usize, std::io::Error> {
        let json = format!(
            "{{\"request\":\"sender data\",\"data\":[{}]}}",
            self.zabbix_item_list
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{{\"host\":\"{}\",\"key\":\"{}\",\"value\":\"{}\"}}",
                        self.zabbix_item_host_name,
                        key,
                        value
                    )
                })
                .collect::<Vec<String>>()
                .join(",")
        );
        let json_len = json.len();
        let mut rem_len = json_len;
        // let mut packet_len = 0;
        for i in 0..8 {
            self.zabbix_packet[5 + i] = (rem_len % 256) as u8;
            rem_len = rem_len / 256;
        }
        self.zabbix_packet[0] = 'Z' as u8;
        self.zabbix_packet[1] = 'B' as u8;
        self.zabbix_packet[2] = 'X' as u8;
        self.zabbix_packet[3] = 'D' as u8;
        self.zabbix_packet[4] = 0x01;
        let json_bytes = json.as_bytes();
        self.zabbix_packet[13..13 + json_len].copy_from_slice(json_bytes);
        let packet_len = 13 + json_len;
        // Don't print raw binary packet to console as it contains control characters
        // that can break WebSocket connections
        let request_message = format!("Request JSON: {}", json);
        println!("{}", request_message);
        add_message(request_message);
        add_message(format!("Zabbix: Sending {} data items to server", self.zabbix_item_list.len()));
        Ok(packet_len)
    }
}

#[get("/favicon.ico")]
async fn favicon() -> Result<HttpResponse, Error> {
    let pixel =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVQI12P4//8/AAX+Av7czFnnAAAAAElFTkSuQmCC";
    let decoded = base64
        ::decode(pixel)
        .map_err(|_| {
            Error::from(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Failed to decode base64 image"
                )
            )
        })?;
    let body = actix_web::web::Bytes::copy_from_slice(&decoded);
    Ok(HttpResponse::Ok().content_type("image/png").body(body))
}

#[get("/")]
async fn index() -> HttpResponse {
    let html =
        format!(r#"<html>
        <body>
        <h1>Welcome to zbx-np {}</h1>
        <ul>
        <li> <a href="/console">Console</a>
        </ul>
        </body>
        </html>"#, APP_VERSION);

    HttpResponse::Ok().content_type("text/html").body(html)
}

#[get("/console")]
async fn console() -> HttpResponse {
    HttpResponse::Ok().content_type("text/html").body(HTML)
}

async fn validator(
    req: ServiceRequest,
    credentials: BasicAuth
) -> Result<ServiceRequest, (Error, ServiceRequest)> {
    let settings_login = CONFIG_JSON["settings"]["http"]["login"].as_str().unwrap().to_string();
    let settings_passw = CONFIG_JSON["settings"]["http"]["password"].as_str().unwrap().to_string();

    if
        credentials.user_id().eq(&settings_login) &&
        credentials.password().unwrap().eq(&settings_passw)
    {
        // eprintln!("{credentials:?}");
        Ok(req)
    } else {
        Err((ErrorUnauthorized("unauthorized"), req))
    }
}

pub async fn run_server() -> std::io::Result<()> {
    println!("zbx-np {}. ©All rights in reserve.", APP_VERSION);

    let port = CONFIG_JSON["settings"]["http"]["port"].as_u64().unwrap_or(7000);
    
    // Initialize MQTT state from config immediately
    let mqtt_enable = CONFIG_JSON["settings"]["mqtt"]["enabled"].as_bool().unwrap();
    let mqtt_url = CONFIG_JSON["settings"]["mqtt"]["url"].as_str().unwrap_or("");
    let mqtt_topic = CONFIG_JSON["settings"]["mqtt"]["topic"].as_str().unwrap_or("");
    
    // Initialize MQTT state immediately to avoid race conditions
    {
        let mut mqtt_state = MQTT_STATE.lock().unwrap();
        mqtt_state.enabled = mqtt_enable;
        mqtt_state.url = mqtt_url.to_string();
        mqtt_state.topic = mqtt_topic.to_string();
        mqtt_state.status = if mqtt_enable { "starting".to_string() } else { "disabled".to_string() };
        mqtt_state.last_updated = chrono::Local::now();
        println!("DEBUG: Initial MQTT state initialized: enabled={}, status='{}'", mqtt_enable, mqtt_state.status);
    }
    
    // Initialize stats
    let stats = Arc::new(Mutex::new(Stats::default()));
    
    // Create app state
    let app_state = AppState {
        stats,
    };

    // Start MQTT client in background if enabled
    if mqtt_enable {
        let stats_clone = app_state.stats.clone();
        let (shutdown_tx, shutdown_rx): (Sender<()>, Receiver<()>) = unbounded();
        
        // Store the shutdown handle
        {
            let mut handle = MQTT_HANDLE.lock().unwrap();
            *handle = Some(shutdown_tx);
        }
        
        tokio::spawn(async move {
            if let Err(e) = mqtt_connect(stats_clone, shutdown_rx).await {
                eprintln!("MQTT connection error: {}", e);
                // Broadcast error status on complete failure
                broadcast_mqtt_status(true, "error", mqtt_url, mqtt_topic);
            }
        });
    }
    // Note: No need to broadcast status here since it's already initialized in MQTT_STATE above

    // Start WebSocket server in background
    thread::spawn(move || {
        if let Err(e) = ws() {
            eprintln!("WebSocket server error: {:?}", e);
        }
    });
    
    // Start the HTTP server
    HttpServer::new(move || {
        let auth = HttpAuthentication::basic(validator);
        
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();
        
        App::new()
            .app_data(web::Data::new(app_state.clone()))
            .wrap(cors)
            // Public endpoints
            .service(index)
            .service(console)
            .service(favicon)
            // Authentication endpoints (no auth required for login)
            .route("/api/auth/login", web::post().to(api::login))
            .route("/api/auth/logout", web::post().to(api::logout))
            // Configuration API endpoints (simplified - no auth for demo)
            .route("/api/config", web::get().to(api::get_config))
            .route("/api/config", web::put().to(api::update_config))
            .route("/api/config/test", web::post().to(api::test_config))
            // Statistics API endpoints
            .route("/api/stats", web::get().to(api::get_stats))
            .route("/api/stats/history", web::get().to(api::get_stats_history))
            .route("/api/stats/realtime", web::get().to(api::get_realtime_stats))
            // MQTT status endpoint
            .route("/api/mqtt/status", web::get().to(api::get_mqtt_status))
            // User management API endpoints (simplified)
            .route("/api/users", web::get().to(api::get_users))
            .route("/api/users", web::post().to(api::create_user))
            .route("/api/users/{id}", web::delete().to(api::delete_user))
            // Token management API endpoints (simplified)
            .route("/api/tokens", web::get().to(api::get_tokens))
            .route("/api/tokens", web::post().to(api::create_token))
            .route("/api/tokens/{id}", web::delete().to(api::delete_token))
            // Management endpoints
            .route("/api/restart", web::post().to(api::restart_service))
            .route("/api/logs", web::get().to(api::get_logs))
            // Data ingestion endpoints (with Basic Auth for backward compatibility)
            .service(
                web::scope("")
                    .wrap(auth)
                    .service(zabbix_handler)
                    .service(zabbix_post_handler)
            )
    })
        .bind(format!("0.0.0.0:{}", port))?
        .run().await
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    run_server().await
}

fn print_time_date() -> String {
    let current_datetime = Local::now();
    let formatted_datetime = current_datetime.format("[%H:%M:%S %d-%m-%Y]").to_string();
    // println!("\n{}", formatted_datetime);
    formatted_datetime
}

#[get("/zabbix")]
async fn zabbix_handler(req: HttpRequest, query: web::Query<UrlQuery>) -> HttpResponse {
    let message = print_time_date();
    println!("\n{}", message);
    add_message(message.to_string());
    
    if let Some(remote_addr) = req.peer_addr() {
        if let Some(ip_address) = remote_addr.ip().to_string().split(':').next() {
            println!("Received data from HTPP via GET: {}", ip_address);
            let message = format!("Received data from HTPP via GET: {}", ip_address);
            add_message(message.to_string());
        } else {
            println!("Unable to extract the IP address");
        }
    }
    else {
        println!("Unable to retrieve the remote IP address");
    }
    
    let data: Data = serde_json
        ::from_str(&query.data)
        .unwrap_or_else(|_| panic!("Failed to parse data"));

    let response_json =
        json!({
        "zabbix_server": data.zabbix_server,
        "item_host_name": data.item_host_name,
        "item": data.item,
    });
    println!("{}", response_json);
    
    let message = response_json.clone();
    add_message(message.to_string());

    let show_result = send_to_zabbix(&response_json.to_string());
    let decoded_show_result = match show_result {
        Ok(show_result) => decode_unicode_escape_sequences(&show_result),
        Err(err) => {
            let error_message = format!("Error sending data to Zabbix server: {}", err);
            eprintln!("{}", error_message);
            add_message(error_message.clone());
            send_message(&error_message);
            // Create an error response JSON
            let error_response =
                json!({
                "error": "Failed to send data to Zabbix server",
                "details": err.to_string(),
            });
            return HttpResponse::InternalServerError().json(error_response);
        }
    };
    let mut response_data =
        json!({
        "data": response_json,
        "result": decoded_show_result
    });
    // Convert the "show_result" field to a JSON value
    if let Some(show_result_value) = response_data.get_mut("result") {
        if let Some(show_result_str) = show_result_value.as_str() {
            if let Ok(show_result_json) = serde_json::from_str(show_result_str) {
                *show_result_value = Value::Object(show_result_json);
            }
        }
    }
    HttpResponse::Ok().json(response_data)
}

#[post("/zabbix")]
async fn zabbix_post_handler(req: HttpRequest, body: web::Json<Data>) -> HttpResponse {
    let message = print_time_date();
    println!("\n{}", message);
    add_message(message.to_string());
    if let Some(remote_addr) = req.peer_addr() {
        if let Some(ip_address) = remote_addr.ip().to_string().split(':').next() {
            println!("Received data from HTPP via POST: {}", ip_address);
            let message = format!("Received data from HTPP via POST: {}", ip_address);
            add_message(message.to_string());
        } else {
            println!("Unable to extract the IP address");
        }
    }
    else {
        println!("Unable to retrieve the remote IP address");
    }
    let response_json =
        json!({
        "zabbix_server": body.zabbix_server,
        "item_host_name": body.item_host_name,
        "item": body.item,
    });
    println!("{}", response_json);

    let message = response_json.clone();
    add_message(message.to_string());

    let show_result = send_to_zabbix(&response_json.to_string());
    let decoded_show_result = match show_result {
        Ok(show_result) => decode_unicode_escape_sequences(&show_result),
        Err(err) => {
            let error_message = format!("Error sending data to Zabbix server: {}", err);
            eprintln!("{}", error_message);
            add_message(error_message.clone());
            send_message(&error_message);
            // Create an error response JSON
            let error_response =
                json!({
                "error": "Failed to send data to Zabbix server",
                "details": err.to_string(),
            });
            return HttpResponse::InternalServerError().json(error_response);
        }
    };

    let mut response_data =
        json!({
        "data": response_json,
        "result": decoded_show_result
    });
    // Convert the "show_result" field to a JSON value
    if let Some(show_result_value) = response_data.get_mut("result") {
        if let Some(show_result_str) = show_result_value.as_str() {
            let show_result_json: Value = serde_json
                ::from_str(show_result_str)
                .unwrap_or_else(|_| panic!("Failed to parse show_result as JSON"));
            *show_result_value = show_result_json;
        }
    }
    HttpResponse::Ok().json(response_data)
}

fn decode_unicode_escape_sequences(input: &str) -> String {
    let prefix = "ZBXD\u{0001}Z\u{0000}\u{0000}\u{0000}\u{0000}\u{0000}\u{0000}\u{0000}";
    let stripped_input = input.strip_prefix(prefix).unwrap_or(input);
    let mut decoded = String::new();
    let mut chars = stripped_input.chars().fuse();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some('u') = chars.next() {
                let unicode_sequence: String = chars.by_ref().take(4).collect();
                if let Ok(unicode_value) = u32::from_str_radix(&unicode_sequence, 16) {
                    if let Some(unicode_char) = std::char::from_u32(unicode_value) {
                        decoded.push(unicode_char);
                    } else {
                        decoded.push_str(&format!("\\u{}", unicode_sequence));
                    }
                } else {
                    decoded.push_str(&format!("\\u{}", unicode_sequence));
                }
                continue;
            }
        }
        decoded.push(ch);
    }
    decoded
}

fn send_to_zabbix(response_json: &str) -> Result<String, std::io::Error> {
    let response_data: serde_json::Value = serde_json::from_str(response_json).unwrap();
    let zabbix_server = response_data["zabbix_server"].as_str().unwrap();
    let zabbix_server_addr = zabbix_server.parse().unwrap();
    let zabbix_item_host_name = response_data["item_host_name"]
        .as_str()
        .unwrap_or_else(|| panic!("Failed to extract item_host_name"))
        .to_string();
    let items = response_data["item"].as_array().unwrap();

    let mut zabbix_sender = ZabbixSender::new(zabbix_server_addr, zabbix_item_host_name);
    for item in items {
        let item_name = item["key"].as_str().unwrap();
        let item_value = match &item["value"] {
            Value::Number(n) => {
                if let Some(int_value) = n.as_i64() {
                    int_value.to_string()
                } else if let Some(float_value) = n.as_f64() {
                    float_value.to_string()
                } else {
                    panic!("Invalid value type for item: {}", item_name);
                }
            }
            _ => panic!("Invalid value type for item: {}", item_name),
        };
        zabbix_sender.add_item(item_name.to_string(), item_value);
    }
    let show_result = zabbix_sender.send()?; // Propagate the error from zabbix_sender.send()
    // Handle the show_result value as needed
    println!("Result = {}", show_result);
    add_message(format!("Zabbix Result: {}", show_result));

    Ok(show_result) // Return the show_result value
}

async fn mqtt_connect(stats: Arc<Mutex<Stats>>, shutdown_rx: Receiver<()>) -> Result<(), Box<dyn std::error::Error>> {
    // Read config from file to get latest settings
    let config_content = std::fs::read_to_string("config.json")?;
    let config: serde_json::Value = serde_json::from_str(&config_content)?;
    
    let period = config["settings"]["mqtt"]["period"].as_u64().unwrap_or(10) * 1000;
    let period_duration = Duration::from_millis(period);
    let mut zabbix_last_msg = Instant::now() - period_duration - Duration::from_millis(1000);

    let host = config["settings"]["mqtt"]["url"].as_str().unwrap().to_string();
    let zabbix_topic = config["settings"]["mqtt"]["topic"].as_str().unwrap();
    
    println!("Connecting to MQTT broker: '{}'", host);
    add_message(format!("MQTT: Connecting to {}", host));
    
    // Update status to connecting now that we're actually attempting connection
    broadcast_mqtt_status(true, "connecting", &host, zabbix_topic);

    let config_id = config["settings"]["mqtt"]["id"].as_str().unwrap();
    let client_id = format!("zbx-np-{}", config_id);
    println!("MQTT Client ID: {}", client_id);

    // Parse MQTT broker URL
    let url = url::Url::parse(&host)?;
    let mqtt_host = url.host_str().unwrap_or("localhost");
    let mqtt_port = url.port().unwrap_or(if url.scheme() == "mqtts" { 8883 } else { 1883 });

    // Create MQTT options
    let mut mqttoptions = MqttOptions::new(client_id, mqtt_host, mqtt_port);
    
    // Set credentials if provided
    let login = config["settings"]["mqtt"]["login"].as_str().unwrap_or("");
    let password = config["settings"]["mqtt"]["password"].as_str().unwrap_or("");
    
    if !login.is_empty() {
        mqttoptions.set_credentials(login, password);
    }

    // Configure connection settings (enhanced from working example)
    mqttoptions.set_keep_alive(Duration::from_secs(30));
    mqttoptions.set_clean_session(false);
    mqttoptions.set_max_packet_size(128 * 1024, 128 * 1024);

    // Enable TLS if using mqtts with proper CA certificate validation
    // Using TlsConfiguration::default() for automatic system CA certificate validation
    if url.scheme() == "mqtts" {
        println!("Enabling TLS for MQTT connection with automatic CA validation");
        add_message("MQTT: Enabling TLS encryption with system CA certificates".to_string());
        mqttoptions.set_transport(Transport::Tls(TlsConfiguration::default()));
        println!("TLS configuration applied successfully for secure MQTT connection");
    }

    // Create MQTT client with enhanced configuration
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

    // Subscribe to topic with enhanced logging
    println!("Subscribing to MQTT topic: {} with QoS AtLeastOnce", zabbix_topic);
    match client.subscribe(zabbix_topic, QoS::AtLeastOnce).await {
        Ok(_) => {
            println!("Successfully subscribed to MQTT topic: {}", zabbix_topic);
            add_message(format!("MQTT: Subscribed to topic {}", zabbix_topic));
            // Don't update status here - wait for actual connection acknowledgment
        }
        Err(e) => {
            eprintln!("Failed to subscribe to MQTT topic {}: {}", zabbix_topic, e);
            add_message(format!("MQTT Error: Failed to subscribe - {}", e));
            // Broadcast error status
            broadcast_mqtt_status(true, "error", &host, zabbix_topic);
            return Err(Box::new(e));
        }
    }

    println!("MQTT client subscription completed, waiting for connection...");
    add_message("MQTT: Waiting for connection acknowledgment".to_string());
    
    let mut reconnect_attempts = 0;
    const MAX_RECONNECT_ATTEMPTS: u32 = 5;

    // Handle MQTT events with shutdown capability
    loop {
        // Use a non-blocking receive with a timeout or poll in a loop
        // for crossbeam_channel. Here, we'll just check if a message is available.
        if let Ok(_) = shutdown_rx.try_recv() {
            println!("MQTT: Received shutdown signal, disconnecting...");
            add_message("MQTT: Service stopped by configuration change".to_string());
            // Broadcast stopped status
            broadcast_mqtt_status(false, "stopped", &host, zabbix_topic);
            return Ok(());
        }

        tokio::select! {
            // Handle MQTT events
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        reconnect_attempts = 0; // Reset counter on successful message
                        let topic = &publish.topic;
                        let payload = String::from_utf8_lossy(&publish.payload);
                        
                        if topic == zabbix_topic {
                            let now = Instant::now();
                    if now - zabbix_last_msg > period_duration {
                        let message = print_time_date();
                        println!("\n{}", message);
                        add_message(message.to_string());
                        
                        println!("Received MQTT message from topic: {}", topic);
                        println!("Payload: {}", payload);
                        add_message(format!("MQTT: {}", payload));
                        
                        // Update stats
                        {
                            let mut stats = stats.lock().unwrap();
                            stats.mqtt_messages += 1;
                        }

                        // Try to parse as JSON and send to Zabbix
                        if payload.trim().is_empty() {
                            println!("Received empty MQTT payload, skipping");
                            continue;
                        }
                        
                        match serde_json::from_str::<Data>(&payload) {
                            Ok(data) => {
                                // Validate required fields
                                if data.zabbix_server.is_empty() {
                                    println!("Invalid MQTT payload: missing zabbix_server");
                                    add_message("MQTT Error: Missing zabbix_server field".to_string());
                                    continue;
                                }
                                
                                if data.item_host_name.is_empty() {
                                    println!("Invalid MQTT payload: missing item_host_name");
                                    add_message("MQTT Error: Missing item_host_name field".to_string());
                                    continue;
                                }
                                
                                if data.item.is_empty() {
                                    println!("Invalid MQTT payload: no items specified");
                                    add_message("MQTT Error: No items in payload".to_string());
                                    continue;
                                }
                                
                                let response_json = json!({
                                    "zabbix_server": data.zabbix_server,
                                    "item_host_name": data.item_host_name,
                                    "item": data.item,
                                });
                                
                                match send_to_zabbix(&response_json.to_string()) {
                                    Ok(result) => {
                                        let decoded_result = decode_unicode_escape_sequences(&result);
                                        println!("Zabbix result: {}", decoded_result);
                                        add_message(format!("Zabbix: {}", decoded_result));
                                        
                                        // Update stats
                                        {
                                            let mut stats = stats.lock().unwrap();
                                            stats.zabbix_sends += 1;
                                            stats.successful_requests += 1;
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Error sending to Zabbix: {}", e);
                                        add_message(format!("Zabbix Error: {}", e));
                                        
                                        // Update stats
                                        {
                                            let mut stats = stats.lock().unwrap();
                                            stats.failed_requests += 1;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                println!("Failed to parse MQTT payload as JSON: {}", e);
                                add_message(format!("Parse Error: {}", e));
                            }
                        }
                        
                                        zabbix_last_msg = Instant::now();
                                    }
                                }
                            }
                            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                                println!("MQTT: Connection acknowledged");
                                add_message("MQTT: Client connected and ready".to_string());
                                broadcast_mqtt_status(true, "running", &host, zabbix_topic);
                                reconnect_attempts = 0;
                            }
                            Ok(Event::Incoming(Packet::Disconnect)) => {
                                println!("MQTT: Received disconnect packet");
                                add_message("MQTT: Disconnected by broker".to_string());
                                broadcast_mqtt_status(true, "disconnected", &host, zabbix_topic);
                            }
                            Ok(_) => {
                                // Handle other events (ping, suback, etc.)
                            }
                            Err(e) => {
                                reconnect_attempts += 1;
                                eprintln!("MQTT connection error (attempt {}): {}", reconnect_attempts, e);
                                add_message(format!("MQTT Error ({}): {}", reconnect_attempts, e));
                                
                                // Always show disconnected on first error, then reconnecting on subsequent errors
                                if reconnect_attempts == 1 {
                                    broadcast_mqtt_status(true, "disconnected", &host, zabbix_topic);
                                } else {
                                    broadcast_mqtt_status(true, "reconnecting", &host, zabbix_topic);
                                }
                                
                                if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                                    eprintln!("Max reconnection attempts reached. Stopping MQTT client.");
                                    add_message("MQTT: Max reconnection attempts reached. Client stopped.".to_string());
                                    broadcast_mqtt_status(true, "error", &host, zabbix_topic);
                                    return Err(Box::new(e));
                                }
                                
                                // Exponential backoff for reconnection
                                let delay_seconds = std::cmp::min(5 * 2_u64.pow(reconnect_attempts - 1), 60);
                                println!("Attempting to reconnect in {} seconds...", delay_seconds);
                                
                                // Show reconnecting status during delay
                                if reconnect_attempts > 1 {
                                    broadcast_mqtt_status(true, "reconnecting", &host, zabbix_topic);
                                }
                                
                                time::sleep(Duration::from_secs(delay_seconds)).await;
                                
                                // Re-subscribe after reconnection
                                if let Err(e) = client.subscribe(zabbix_topic, QoS::AtLeastOnce).await {
                                    eprintln!("Failed to re-subscribe: {}", e);
                                    add_message(format!("MQTT Re-subscribe Error: {}", e));
                                } else {
                                    println!("Successfully re-subscribed to topic: {}", zabbix_topic);
                                    add_message(format!("MQTT: Re-subscribed to {}", zabbix_topic));
                                    // Set status back to connecting after successful re-subscription
                                    broadcast_mqtt_status(true, "connecting", &host, zabbix_topic);
                                }
                            }
                        }
                    }
                }
            }
    }

fn ws() -> std::io::Result<()> {
    let clients = Arc::new(Mutex::new(Vec::new()));

    // Start listening for WebSocket connections
    let ws_server = Server::bind("0.0.0.0:2794").unwrap();

    for connection in ws_server.filter_map(Result::ok) {
        // Clone the Arc for each connection
        let clients = Arc::clone(&clients);

        // Spawn a new thread for each connection.
        thread::spawn(move || {
            if !connection.protocols().contains(&"rust-websocket".to_string()) {
                connection.reject().unwrap();
                return;
            }

            let mut client = match connection.use_protocol("rust-websocket").accept() {
                Ok(client) => client,
                Err(err) => {
                    eprintln!("Error accepting WebSocket connection: {:?}", err);
                    return;
                }
            };

            let ip = client.peer_addr().unwrap();

            println!("Connection from {}", ip);

            let message = Message::text("Connected to server");
            if let Err(err) = client.send_message(&message) {
                eprintln!("Error sending initial message to client {}: {:?}", ip, err);
                return;
            }

            let (mut receiver, sender) = client.split().unwrap();

            // Wrap the sender in an Arc<Mutex<Sender>>
            let client_sender = Arc::new(Mutex::new(sender));

            // Store the client_sender in the global SENDERS variable
            {
                let mut senders = SENDERS.lock().unwrap();
                senders.push(client_sender.clone());
            }

            // Add the client_sender to the clients vector
            {
                let mut clients = clients.lock().unwrap();
                clients.push(client_sender.clone());
            }
            
            // Send current MQTT status to the new client
            send_current_mqtt_status_to_client(&client_sender);
            
            // Send recent messages to the new client
            {
                let messages = get_messages();
                println!("Sending {} messages to new WebSocket client", messages.len());
                for message in messages.iter().take(20) { // Send last 20 messages with newest first
                    // Filter out binary or problematic messages and ensure UTF-8 compatibility
                    // Allow Russian/Cyrillic text but exclude control characters that could break WebSocket
                    let clean_message = message.chars()
                        .filter(|c| {
                            // Allow printable characters, whitespace, and most Unicode characters
                            // but exclude control characters (except newlines, tabs, carriage returns)
                            !c.is_control() || matches!(c, '\n' | '\r' | '\t')
                        })
                        .take(200) // Limit message length to prevent WebSocket issues
                        .collect::<String>();
                    
                    if !clean_message.trim().is_empty() {
                        println!("Sending message: {}", clean_message.chars().take(50).collect::<String>());
                        let msg = Message::text(clean_message);
                        if let Err(err) = client_sender.lock().unwrap().send_message(&msg) {
                            eprintln!("Error sending recent message to client {}: {:?}", ip, err);
                        }
                    } else {
                        println!("Filtered out problematic message: {}", message.chars().take(50).collect::<String>());
                    }
                }
            }

            for message in receiver.incoming_messages() {
                let message = match message {
                    Ok(msg) => msg,
                    Err(err) => {
                        eprintln!("Error receiving message from client {}: {}", ip, err);

                        // Remove the client_sender from the global SENDERS vector
                        {
                            let mut senders = SENDERS.lock().unwrap();
                            if
                                let Some(position) = senders
                                    .iter()
                                    .position(|sender| Arc::ptr_eq(sender, &client_sender))
                            {
                                senders.remove(position);
                            }
                        }

                        // Remove the client_sender from the clients vector
                        {
                            let mut clients = clients.lock().unwrap();
                            if
                                let Some(position) = clients
                                    .iter()
                                    .position(|client| Arc::ptr_eq(client, &client_sender))
                            {
                                clients.remove(position);
                            }
                        }

                        break;
                    }
                };

                match message {
                    OwnedMessage::Close(_) => {
                        // Remove the client_sender from the global SENDERS vector
                        {
                            let mut senders = SENDERS.lock().unwrap();
                            if
                                let Some(position) = senders
                                    .iter()
                                    .position(|sender| Arc::ptr_eq(sender, &client_sender))
                            {
                                senders.remove(position);
                            }
                        }

                        // Remove the client_sender from the clients vector
                        {
                            let mut clients = clients.lock().unwrap();
                            if
                                let Some(position) = clients
                                    .iter()
                                    .position(|client| Arc::ptr_eq(client, &client_sender))
                            {
                                clients.remove(position);
                            }
                        }

                        let message = Message::close();
                        if let Err(err) = client_sender.lock().unwrap().send_message(&message) {
                            eprintln!("Error sending close message to client {}: {:?}", ip, err);
                        }
                        println!("Client {} disconnected", ip);
                        break;
                    }
                    OwnedMessage::Ping(data) => {
                        let message = Message::pong(data);
                        if let Err(err) = client_sender.lock().unwrap().send_message(&message) {
                            eprintln!("Error sending pong message to client {}: {:?}", ip, err);
                        }
                    }
                    OwnedMessage::Text(ref text) if text == "last" => {
                        println!("Received 'last' message from client: {}", text);
                        unsafe {
                            // Access the global_messages variable
                            send_message("");
                            for message in &GLOBAL_MESSAGES {
                                send_message(message);
                            }
                        }
                    }
                    _ => {
                        // Send the message to all clients
                        let senders = SENDERS.lock().unwrap();
                        for sender in &*senders {
                            if let Err(err) = sender.lock().unwrap().send_message(&message) {
                                eprintln!("Error sending message to client: {:?}", err);
                            }
                        }
                    }
                }
            }

            // Remove the client_sender from the clients vector
            {
                let mut clients = clients.lock().unwrap();
                if
                    let Some(position) = clients
                        .iter()
                        .position(|client| Arc::ptr_eq(client, &client_sender))
                {
                    clients.remove(position);
                }
            }
        });
    }
    Ok(())
}
