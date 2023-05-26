# zbx-np
It's zabbix relay that can send data to your zabbix server using post or get requests.

# Use get request: 
http://localhost:8000/zabbix?data={"item":[{"key":"voltage","value":544},{"key":"potenciometr","value":4459},{"key":"button","value":4459}],"item_host_name":"esp8266-ar","zabbix_server":"192.168.243.229:10051"}

# Use post request: 
http://localhost:8000/zabbix 

with body:{"item":[{"key":"voltage","value":544},{"key":"potenciometr","value":4459},{"key":"button","value":4459}],"item_host_name":"esp8266-ar","zabbix_server":"192.168.243.229:10051"}

# Settings
All settings are stored in config.json, edit it for yours gole:

"port":8000 - http port

"login": "admin" - Basic auth credential login

"password": "admin" - Basic auth credential password
