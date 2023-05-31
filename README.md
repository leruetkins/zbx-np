# zbx-np
It's zabbix relay that can send data to your zabbix server using post or get requests. Now everything that can send http requests can send data to **Zabbix**. Now zbx-np supports get data form MQTT. For this purpose it uses **paho_mqtt** library witch require libcrypto-3-x64.dll and libssl-3-x64.dll libraries from OpenSSL project. Just put tehese files near the app.

**zbx-np** configured to use **hivemq**(https://www.hivemq.com/) broker. So you can insert your credentials and settings to config.json. Maybe other brokers, local or cloud will work too, but zbx-np specifically works with hivemq. You can modify code to use another mqtt library, for example **rumqtt**(https://github.com/bytebeamio/rumqtt) to do not use additionals libraries.

# How to use:

Before use, create a host with an zabbix trapper item:

https://www.zabbix.com/documentation/6.0/en/manual/config/items/itemtypes/trapper

# Use GET request: 
http://localhost:8000/zabbix?data={"item":[{"key":"voltage","value":54.4},{"key":"potenciometr","value":4459},{"key":"button","value":4459}],"item_host_name":"esp8266-ar","zabbix_server":"192.168.243.229:10051"}

# Use POST request: 
http://localhost:8000/zabbix 

With Body:

{"item":[{"key":"voltage","value":54.4},{"key":"potenciometr","value":4459},{"key":"button","value":4459}],"item_host_name":"esp8266-ar","zabbix_server":"192.168.243.229:10051"}

# Settings
All settings are stored in config.json, edit it for yours gole:

"port":8000 - http port

"login": "admin" - Basic auth credential login

"password": "admin" - Basic auth credential password

You can install bc-np as system service using **nssm** https://nssm.cc/download.
