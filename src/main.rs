use std::net::{TcpStream, SocketAddr};
use std::io::{Write, Read};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use once_cell::sync::Lazy; 
use actix_web::{get,web, HttpResponse, Result, App, HttpServer};
use actix_web::{dev::ServiceRequest, Error};
use actix_web_httpauth::{extractors::basic::BasicAuth, middleware::HttpAuthentication};
use actix_web::error::ErrorUnauthorized;
use serde_json::json;

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
    value: i32,
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


    pub fn send(&mut self) -> Result<(), std::io::Error> {
        let mut ret_value = 1;
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
            let result = std::str::from_utf8(&buf[..bytes_read]).unwrap();
            println!("result = {}", result);
            ret_value = 0;
        } else {
            println!("No result");
        }

        if ret_value != 0 {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Send operation failed",
            ))
        } else {
            Ok(())
        }
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

        println!("request = {}", String::from_utf8_lossy(&self.zabbix_packet[..packet_len]));

        Ok(packet_len)
    }
}

#[get("/favicon.ico")]
async fn favicon() -> Result<HttpResponse, Error> {
    let pixel = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVQI12P4//8/AAX+Av7czFnnAAAAAElFTkSuQmCC";
    let decoded = base64::decode(pixel).map_err(|_| Error::from(std::io::Error::new(std::io::ErrorKind::InvalidData, "Failed to decode base64 image")))?;
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
    let settings_login = CONFIG_JSON["settings"][0]["login"].as_str().unwrap().to_string();
    let settings_passw = CONFIG_JSON["settings"][0]["password"].as_str().unwrap().to_string();

    if credentials.user_id().eq(&settings_login) && credentials.password().unwrap().eq(&settings_passw) {
        // eprintln!("{credentials:?}");
        Ok(req)
    } else {
        Err((ErrorUnauthorized("unauthorized"), req))
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    let port = CONFIG_JSON["settings"][0]["port"].as_u64().unwrap_or(8000);
    // Start the config job in a new thread
    // thread::spawn(|| run_config_job());

    // Start the HTTP server
    HttpServer::new(|| {
        let auth = HttpAuthentication::basic(validator);
                App::new()
            .wrap(auth)   
            .service(index)
            .service(favicon)
            .service(zabbix_handler)

    })
    .bind(format!("0.0.0.0:{}", port))?
    .run()
    .await
}

#[get("/zabbix")]
async fn zabbix_handler(query: web::Query<UrlQuery>) -> HttpResponse {
    let data: Data = serde_json::from_str(&query.data)
        .unwrap_or_else(|_| panic!("Failed to parse data"));

    let response_json = json!({
        "zabbix_server": data.zabbix_server,
        "item_host_name": data.item_host_name,
        "items": data.item,
    });

    send_to_zabbix(&response_json.to_string());

    HttpResponse::Ok().json(response_json)
}

fn send_to_zabbix(response_json: &str) {
    
    let response_data: serde_json::Value = serde_json::from_str(response_json).unwrap();

    let zabbix_server = response_data["zabbix_server"].as_str().unwrap();
    let zabbix_server_addr = zabbix_server.parse().unwrap();
    let zabbix_item_host_name = response_data["item_host_name"]
        .as_str()
        .unwrap_or_else(|| panic!("Failed to extract item_host_name"))
        .to_string();
    let items = response_data["items"].as_array().unwrap();

    let mut zabbix_sender = ZabbixSender::new(zabbix_server_addr, zabbix_item_host_name);
    for item in items {
        let item_name = item["key"].as_str().unwrap();
        let item_value = item["value"].as_i64().unwrap().to_string();
        zabbix_sender.add_item(item_name.to_string(), item_value);
    }

    if let Err(err) = zabbix_sender.send() {
        eprintln!("Error sending data to Zabbix server: {}", err);
    }
}

// 1111