# zbx-np
It's zabbix relay that can send data to your zabbix server using **POST** or **GET** requests. Now everything that can send http requests can send data to **Zabbix**. Also **zbx-np** supports get data from **MQTT** using the modern **rumqtt** library.

**zbx-np** is configured to use **HiveMQ** (https://www.hivemq.com) broker by default. You can insert your credentials and settings in config.json. Other MQTT brokers (local or cloud) should also work. The application uses the **rumqtt** library (https://github.com/bytebeamio/rumqtt) which provides excellent performance and avoids OpenSSL compilation issues.

## Features
- ✅ **HTTP Data Ingestion**: Accept data via GET/POST requests
- ✅ **MQTT Support**: Receive data from MQTT brokers using rumqtt
- ✅ **Web Interface**: Modern Vue.js dashboard with dark theme
- ✅ **TLS Support**: Secure MQTT connections with proper TLS configuration
- ✅ **Auto-Reconnection**: Intelligent reconnection with exponential backoff
- ✅ **Real-time Monitoring**: WebSocket-based real-time activity feed
- ✅ **Configuration Management**: Web-based configuration editor

## MQTT Configuration with rumqtt

### Why rumqtt?
- **No OpenSSL Dependencies**: Avoids compilation issues on Windows
- **Better Performance**: Async/await support with tokio
- **TLS Support**: Built-in TLS with proper ALPN protocol negotiation  
- **Auto-Reconnection**: Intelligent reconnection with exponential backoff
- **Better Error Handling**: Comprehensive error reporting and recovery

### MQTT Settings in config.json
```json
{
  "settings": {
    "mqtt": {
      "enabled": true,
      "url": "mqtts://your-broker.hivemq.cloud:8883",
      "id": "your-client-id",
      "login": "your-username", 
      "password": "your-password",
      "period": 10,
      "topic": "zabbix/your-topic"
    }
  }
}
```

### MQTT Message Format
Send JSON messages to your MQTT topic with this structure:
```json
{
  "zabbix_server": "127.0.0.1:10051",
  "item_host_name": "your-device-name",
  "item": [
    {
      "key": "system.cpu.util",
      "value": 75.5
    },
    {
      "key": "system.memory.util", 
      "value": 85.2
    }
  ]
}
```

# Compiling issue:
If you have this message while compiling on Windows:

=-=-=-=--=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=

  This perl implementation doesn't produce Windows like paths (with backward
  slash directory separators).  Please use an implementation that matches your
  building platform.
  
=-=-=-=--=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=

  This Perl version: 5.36.0 for x86_64-msys-thread-multi
  
=-=-=-=--=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=-=

Just install https://strawberryperl.com/ and delete or temporary rename C:\Program Files\Git\usr\bin\perl.exe file. Git's perl can't compile openssl libraries on rust.

# How to use:

Before use, create a host with an zabbix trapper item:

https://www.zabbix.com/documentation/6.0/en/manual/config/items/itemtypes/trapper

# Use GET request: 
http://localhost:7000/zabbix?data={"item":[{"key":"voltage","value":54.4},{"key":"potenciometr","value":4459},{"key":"button","value":4459}],"item_host_name":"esp8266-ar","zabbix_server":"192.168.243.229:10051"}

# Use POST request: 
http://localhost:7000/zabbix 

With Body:

{"item":[{"key":"voltage","value":54.4},{"key":"potenciometr","value":4459},{"key":"button","value":4459}],"item_host_name":"esp8266-ar","zabbix_server":"192.168.243.229:10051"}

# Websocket client

Open http://localhost:7000/console to get websocket client to see new messages online.

# Settings
All settings are stored in config.json, edit it for yours gole:

## Configuration Parameters

### HTTP Settings
- **"port": 7000** - HTTP server port
- **"login": "admin"** - Basic auth username for web interface
- **"password": "admin"** - Basic auth password for web interface

### MQTT Settings  
- **"enabled": true** - Enable/disable MQTT functionality
- **"url": "mqtts://broker:8883"** - MQTT broker URL (supports mqtt:// and mqtts://)
- **"id": "client-id"** - Unique MQTT client identifier
- **"login": "username"** - MQTT broker username
- **"password": "password"** - MQTT broker password
- **"period": 10** - Message processing frequency in seconds (0 = no throttling)
- **"topic": "zabbix/topic"** - MQTT topic to subscribe to

### Enhanced Features
- **TLS Support**: Automatic TLS with ALPN protocol negotiation for mqtts:// URLs
- **Auto-Reconnection**: Up to 5 retry attempts with exponential backoff (5s, 10s, 20s, 40s, 60s)
- **Message Validation**: Validates required fields (zabbix_server, item_host_name, items)
- **Connection Monitoring**: Real-time connection status in web interface

# Web Interface

zbx-np now includes a modern web interface for monitoring and configuration:

- **Dashboard**: Real-time monitoring with connection status indicators
- **Configuration**: Web-based editor for MQTT and HTTP settings
- **Statistics**: Performance metrics and activity logs
- **Dark Mode**: Responsive design with light/dark theme toggle

**Access**: http://localhost:7000/ (use admin/admin for login)

## API Endpoints

### Authentication
- `POST /api/auth/login` - Login with credentials
- `POST /api/auth/logout` - Logout

### Configuration
- `GET /api/config` - Get current configuration
- `PUT /api/config` - Update configuration
- `POST /api/config/test` - Test configuration

### Monitoring
- `GET /api/stats` - Get system statistics
- `GET /api/logs` - Get application logs
- `WebSocket :2794` - Real-time activity feed

# Installation

You can install **zbx-np** as system service using **nssm** https://nssm.cc/download.
