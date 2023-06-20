use actix_web::error::ErrorUnauthorized;
use actix_web::HttpRequest;
use actix_web::{dev::ServiceRequest, Error};
use actix_web::{get, post, web, App, HttpResponse, HttpServer, Result};
use actix_web_httpauth::{extractors::basic::BasicAuth, middleware::HttpAuthentication};
use chrono::Local;
use futures::{executor::block_on, stream::StreamExt};
use once_cell::sync::Lazy;
use paho_mqtt as mqtt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Number;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process;
use std::time::Instant;
use std::{env, time::Duration};

use hyper::net::Fresh;
use hyper::server::request::Request;
use hyper::server::response::Response;
use hyper::Server as OtherHttpServer;
use std::sync::{Arc, Mutex};
use std::thread;
use websocket::sync::Server;
use websocket::{Message, OwnedMessage};

use lazy_static::lazy_static;



use websocket::sender::Sender;
use websocket::sender::Writer;

lazy_static! {
    static ref SENDERS: Arc<Mutex<Vec<Arc<Mutex<Writer<TcpStream>>>>>> = Arc::new(Mutex::new(Vec::new()));
}

const HTML: &'static str = include_str!("websockets.html");

const QOS: &[i32] = &[1, 1];

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
static CONFIG_JSON: Lazy<serde_json::Value> = Lazy::new(|| {
    let config = fs::read_to_string("config.json").expect("Unable to read config");
    serde_json::from_str(&config).expect("Invalid JSON format")
});

const ZABBIX_MAX_LEN: usize = 300;
const ZABBIX_TIMEOUT: u64 = 1000;

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

        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Send operation failed",
        ))
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
                        self.zabbix_item_host_name, key, value
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

        println!(
            "Request = {}",
            String::from_utf8_lossy(&self.zabbix_packet[..packet_len])
        );
        let message=String::from_utf8_lossy(&self.zabbix_packet[..packet_len]);
        send_message(&message);

        Ok(packet_len)
    }
}

#[get("/favicon.ico")]
async fn favicon() -> Result<HttpResponse, Error> {
    let pixel = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVQI12P4//8/AAX+Av7czFnnAAAAAElFTkSuQmCC";
    let decoded = base64::decode(pixel).map_err(|_| {
        Error::from(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Failed to decode base64 image",
        ))
    })?;
    let body = actix_web::web::Bytes::copy_from_slice(&decoded);
    Ok(HttpResponse::Ok().content_type("image/png").body(body))
}

#[get("/")]
async fn index() -> HttpResponse {
    let html = format!(
        r#"<html>
        <h1>Welcome to zbx-np {}</h1>
        </html>"#,
        APP_VERSION
    );

    HttpResponse::Ok().content_type("text/html").body(html)
}

async fn validator(
    req: ServiceRequest,
    credentials: BasicAuth,
) -> Result<ServiceRequest, (Error, ServiceRequest)> {
    let settings_login = CONFIG_JSON["settings"]["http"]["login"]
        .as_str()
        .unwrap()
        .to_string();
    let settings_passw = CONFIG_JSON["settings"]["http"]["password"]
        .as_str()
        .unwrap()
        .to_string();

    if credentials.user_id().eq(&settings_login)
        && credentials.password().unwrap().eq(&settings_passw)
    {
        // eprintln!("{credentials:?}");
        Ok(req)
    } else {
        Err((ErrorUnauthorized("unauthorized"), req))
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("zbx-np {}. ©All rights in reserve.", APP_VERSION);

    let port = CONFIG_JSON["settings"]["http"]["port"]
        .as_u64()
        .unwrap_or(7000);
    // Start the config job in a new thread
    let mqtt_enable = CONFIG_JSON["settings"]["mqtt"]["enabled"]
        .as_bool()
        .unwrap();
    // if mqtt_enable {
    //     tokio::spawn(async {
    //         mqtt_connect().await;
    //     });
    // }
    if mqtt_enable {
        mqtt_connect().await;
    }

    thread::spawn(move || {
        block_on(async {
            ws().await;
        });
    });
    // Start the HTTP server
    HttpServer::new(|| {
        let auth = HttpAuthentication::basic(validator);
        App::new()
            .wrap(auth)
            .service(index)
            .service(favicon)
            .service(zabbix_handler)
            .service(zabbix_post_handler)
    })
    .bind(format!("0.0.0.0:{}", port))?
    .run()
    .await
}

fn print_time_date() {
    let current_datetime = Local::now();
    let formatted_datetime = current_datetime.format("%H:%M:%S %d-%m-%Y");
    println!("\n[{}]", formatted_datetime);
}

#[get("/zabbix")]
async fn zabbix_handler(req: HttpRequest, query: web::Query<UrlQuery>) -> HttpResponse {
    print_time_date();
    if let Some(remote_addr) = req.peer_addr() {
        if let Some(ip_address) = remote_addr.ip().to_string().split(':').next() {
            println!("Received data from HTPP via GET: {}", ip_address);
        } else {
            println!("Unable to extract the IP address");
        }
    } else {
        println!("Unable to retrieve the remote IP address");
    }
    let data: Data =
        serde_json::from_str(&query.data).unwrap_or_else(|_| panic!("Failed to parse data"));

    let response_json = json!({
        "zabbix_server": data.zabbix_server,
        "item_host_name": data.item_host_name,
        "item": data.item,
    });
    println!("{}", response_json);

    let show_result = send_to_zabbix(&response_json.to_string());
    let decoded_show_result = match show_result {
        Ok(show_result) => decode_unicode_escape_sequences(&show_result),
        Err(err) => {
            eprintln!("Error sending data to Zabbix server: {}", err);
            // Create an error response JSON
            let error_response = json!({
                "error": "Failed to send data to Zabbix server",
                "details": err.to_string(),
            });
            return HttpResponse::InternalServerError().json(error_response);
        }
    };

    let mut response_data = json!({
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
    print_time_date();
    if let Some(remote_addr) = req.peer_addr() {
        if let Some(ip_address) = remote_addr.ip().to_string().split(':').next() {
            println!("Received data from HTPP via POST: {}", ip_address);
        } else {
            println!("Unable to extract the IP address");
        }
    } else {
        println!("Unable to retrieve the remote IP address");
    }
    let response_json = json!({
        "zabbix_server": body.zabbix_server,
        "item_host_name": body.item_host_name,
        "item": body.item,
    });
    println!("{}", response_json);

    let show_result = send_to_zabbix(&response_json.to_string());
    let decoded_show_result = match show_result {
        Ok(show_result) => decode_unicode_escape_sequences(&show_result),
        Err(err) => {
            eprintln!("Error sending data to Zabbix server: {}", err);
            // Create an error response JSON
            let error_response = json!({
                "error": "Failed to send data to Zabbix server",
                "details": err.to_string(),
            });
            return HttpResponse::InternalServerError().json(error_response);
        }
    };

    let mut response_data = json!({
        "data": response_json,
        "result": decoded_show_result
    });

    // Convert the "show_result" field to a JSON value
    if let Some(show_result_value) = response_data.get_mut("result") {
        if let Some(show_result_str) = show_result_value.as_str() {
            let show_result_json: Value = serde_json::from_str(show_result_str)
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

    Ok(show_result) // Return the show_result value
}

async fn mqtt_connect() {
    let period = CONFIG_JSON["settings"]["mqtt"]["period"].as_u64().unwrap() * 1000;
    let period_duration = Duration::from_millis(period);
    let mut zabbix_last_msg = Instant::now() - period_duration - Duration::from_millis(1000);

    let host = CONFIG_JSON["settings"]["mqtt"]["url"]
        .as_str()
        .unwrap()
        .to_string();
    println!("Connecting to host: '{}'", host);

    let zabbix_topic = CONFIG_JSON["settings"]["mqtt"]["topic"].as_str().unwrap();
    let config_id = CONFIG_JSON["settings"]["mqtt"]["id"].as_str().unwrap();
    let name_id = format!("zbx-np-{}", config_id);
    println!("Client ID: {}", name_id);
    // Create the client. Use an ID for a persistent session.
    // A real system should try harder to use a unique ID.
    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(host)
        .client_id(name_id)
        .finalize();

    // Create the client connection
    let mut cli = mqtt::AsyncClient::new(create_opts).unwrap_or_else(|e| {
        println!("Error creating the client: {:?}", e);
        process::exit(1);
    });

    if let Err(err) = block_on(async {
        // Get message stream before connecting.
        let mut strm = cli.get_stream(25);

        let ssl_opts = mqtt::SslOptionsBuilder::new()
            .enable_server_cert_auth(false)
            .finalize();

        let login = CONFIG_JSON["settings"]["mqtt"]["login"]
            .as_str()
            .unwrap()
            .to_string();

        let password = CONFIG_JSON["settings"]["mqtt"]["password"]
            .as_str()
            .unwrap()
            .to_string();

        let conn_opts = mqtt::ConnectOptionsBuilder::new()
            .ssl_options(ssl_opts)
            .user_name(login)
            .password(password)
            .keep_alive_interval(Duration::from_secs(20))
            .clean_session(false)
            // .will_message(lwt)
            .finalize();

        // Make the connection to the broker
        println!("Connecting to the MQTT server...");
        cli.connect(conn_opts).await?;
        // cli.connect_with_callbacks(conn_opts, on_connect_success, on_connect_failure).await?;
        cli.set_connected_callback(|_cli: &mqtt::AsyncClient| {
            println!("Connected.");
        });

        println!("Subscribing to topics: {}", zabbix_topic);
        cli.subscribe(zabbix_topic, QOS[0]).await?;

        // Just loop on incoming messages.
        println!("Waiting for messages...");

        // Note that we're not providing a way to cleanly shut down and
        // disconnect. Therefore, when you kill this app (with a ^C or
        // whatever) the server will get an unexpected drop and then
        // should emit the LWT message.

        while let Some(msg_opt) = strm.next().await {
            if let Some(msg) = msg_opt {
                // println!("Topic: {}", msg.topic());
                // println!("Payload: {}", msg.payload_str());
                let topic = msg.topic();
                let payload_str = msg.payload_str();
                if topic == zabbix_topic {
                    let now = Instant::now();
                    if (now - zabbix_last_msg) > Duration::from_millis((period).try_into().unwrap())
                    {
                        print_time_date();
                        let data: Result<Data, _> = serde_json::from_str(&payload_str);
                        match data {
                            Ok(ref obj) => {
                                let json_string = serde_json::to_string(&obj);
                                match json_string {
                                    Ok(_) => {
                                    // Ok(string) => {
                                        // println!("{}", string);
                                        // println!(
                                        //     "Received data from MQTT:\n{} - {}",
                                        //     topic, string
                                        // );
                                        println!("Received data from MQTT:");
                                        println!("Topic: {}", msg.topic());
                                        println!("Payload: {}", msg.payload_str());
                                    }
                                    Err(ref e) => {
                                        println!("Failed to convert JSON to string: {}", e);
                                    }
                                }
                            }
                            Err(ref e) => {
                                println!("Failed to parse JSON: {}", e);
                            }
                        }
                        if let Ok(data) = data {
                            let response_json = json!({
                                "zabbix_server": data.zabbix_server,
                                "item_host_name": data.item_host_name,
                                "item": data.item,
                            });
                            let show_result = send_to_zabbix(&response_json.to_string());
                            let decoded_show_result = match &show_result {
                                Ok(result) => decode_unicode_escape_sequences(&result),
                                Err(err) => {
                                    // Handle the error here, e.g., log or display an error message
                                    eprintln!("Error: {:?}", err);
                                    // Provide a default value or return early, depending on your logic
                                    // For example, you can return an empty string:
                                    String::new()
                                }
                            };
                            let mut response_data = json!({
                                "data": response_json,
                                "result": decoded_show_result
                            });
                            // Convert the "show_result" field to a JSON value
                            if let Some(show_result_value) = response_data.get_mut("result") {
                                if let Some(show_result_str) = show_result_value.as_str() {
                                    if let Ok(show_result_json) =
                                        serde_json::from_str(show_result_str)
                                    {
                                        *show_result_value = Value::Object(show_result_json);
                                    }
                                }
                            }
                        } else if let Err(err) = data {
                            eprintln!("Failed to parse payload as JSON object: {}", err);
                            // Handle the parsing error
                        }
                        zabbix_last_msg = Instant::now();
                    }
                }
            } else {
                // A "None" means we were disconnected. Try to reconnect...
                println!("Lost connection. Attempting reconnect.");
                while let Err(err) = cli.reconnect().await {
                    println!("Error reconnecting: {}", err);
                    // For tokio use: tokio::time::delay_for()
                    async_std::task::sleep(Duration::from_millis(1000)).await;
                }
            }
        }

        // Explicit return type for the async block
        Ok::<(), mqtt::Error>(())
    }) {
        eprintln!("{}", err);
    }
}





async fn ws() {
    let clients = Arc::new(Mutex::new(Vec::new()));

    // Start listening for http connections
    thread::spawn(|| {
        let http_server = OtherHttpServer::http("0.0.0.0:8080").unwrap();
        http_server.handle(http_handler).unwrap();
    });

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

            let message = Message::text("Hello");
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

            for message in receiver.incoming_messages() {
                let message = match message {
                    Ok(msg) => msg,
                    Err(err) => {
                        eprintln!("Error receiving message from client {}: {}", ip, err);
                        break;
                    }
                };

                match message {
                    OwnedMessage::Close(_) => {
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
                if let Some(position) = clients.iter().position(|client| Arc::ptr_eq(client, &client_sender)) {
                    clients.remove(position);
                }
            }
        });
    }
}

fn send_message(message: &str) {
    let senders = SENDERS.lock().unwrap();
    for sender in &*senders {
        let message = Message::text(message);
        if let Err(err) = sender.lock().unwrap().send_message(&message) {
            eprintln!("Error sending message: {:?}", err);
        }
    }
}

fn http_handler(_: Request, response: Response<Fresh>) {
    let mut response = response.start().unwrap();
    // Send a client webpage
    response.write_all(HTML.as_bytes()).unwrap();
    response.end().unwrap();
}
//5555
