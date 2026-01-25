# aelys http server

http server written in aelys

## usage

```bash
./aelys run server.aelys -ae.trusted=true
```

server runs on `http://127.0.0.1:8080`

## endpoints

```
GET  /api/hello   - hello message
GET  /api/status  - server stats
GET  /api/time    - current timestamp
POST /api/echo    - echo request body
```

## config

edit `config.aelys` to change host/port/settings

## structure

```
server.aelys          entry point
config.aelys          configuration
lib/
  router.aelys        routing
  static.aelys        static files
  http.aelys          http protocol
  utils.aelys         utilities
public/               static files
```

## requirements

aelys runtime 0.10.0+, trusted mode for network access
