# iss-demo

Tiny demo service that polls the [open-notify ISS position
API](http://api.open-notify.org/iss-now.json) every 5 seconds and writes
the result into geomqtt over its RESP-compatible endpoint.

End result: subscribers on `geo/iss/<z>/<x>/<y>` get a `move` event every
5 seconds as the ISS crosses tiles, and `objects/iss` carries the static
attributes (`type=satellite`, `icon=iss`, `color=white`, `name=...`).

## Run with docker compose

The repository's top-level `docker-compose.yml` already wires this up.
From the repo root:

```sh
docker compose up --build
```

`iss-demo` waits for `geomqtt` to bind, then begins polling.

## Run standalone

Pull the published image from GHCR:

```sh
docker run --rm \
    -e GEOMQTT_HOST=host.docker.internal \
    -e GEOMQTT_PORT=6380 \
    ghcr.io/openfantasymap/geomqtt-iss-demo:latest
```

Or build it locally:

```sh
docker build -t geomqtt-iss-demo examples/iss-demo
docker run --rm \
    -e GEOMQTT_HOST=host.docker.internal \
    -e GEOMQTT_PORT=6380 \
    geomqtt-iss-demo
```

## Configuration

| env var               | default                                     | meaning                           |
|-----------------------|---------------------------------------------|-----------------------------------|
| `GEOMQTT_HOST`        | `geomqtt`                                   | RESP-endpoint host                |
| `GEOMQTT_PORT`        | `6380`                                      | RESP-endpoint port                |
| `GEOMQTT_SET`         | `iss`                                       | Redis GEO set name                |
| `GEOMQTT_OBID`        | `iss`                                       | member id inside the set          |
| `GEOMQTT_OBJ_PREFIX`  | `obj:`                                      | matches `GEOMQTT_OBJECT_KEY_PREFIX` |
| `INTERVAL`            | `5`                                         | poll interval in seconds          |
| `ISS_URL`             | `http://api.open-notify.org/iss-now.json`   | upstream endpoint                 |

## Watching it

```sh
# RESP / GeoJSON read
curl 'http://localhost:8080/objects/iss'
curl 'http://localhost:8080/viewport/iss?bbox=-180,-90,180,90'

# MQTT (TCP)
mosquitto_sub -h localhost -p 1883 -t 'geo/iss/#' -v
```
