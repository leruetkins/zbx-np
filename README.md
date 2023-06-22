# zbx-np
It's zabbix relay that can send data to your zabbix server using **POST** or **GET** requests. Now everything that can send http requests can send data to **Zabbix**. Also **zbx-np** supports get data form **MQTT**.

**zbx-np** configured to use **hivemq** (https://www.hivemq.com) broker. So you can insert your credentials and settings to config.json. Maybe other brokers, local or cloud will work too, but zbx-np specifically works with hivemq. You can modify code to use another mqtt library, for example **rumqtt** (https://github.com/bytebeamio/rumqtt) to do not use additionals libraries.

# Compiling ussue:
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

"port":8000 - http port,

"login": "admin" - Basic auth credential login,

"password": "admin" - Basic auth credential password.

"enabled": true -  enable or disable using MQTT,

"url": "mqtts://address.s2.eu.hivemq.cloud:8883" - your MQTT broker address, it use TLS withot certeficate,

"login": "login" - your MQTT login,

"password":  - your MQTT password,

"period":10 - the frequency in seconds with which data from the topic will be collected,

"topic": "/zabbix/test" - your MQTT topic.

Don't forget to put config.json near zbx-np app.

You can install **zbx-np** as system service using **nssm** https://nssm.cc/download.
