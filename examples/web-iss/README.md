# web-iss — static MapLibre demo

Tiny static site that connects to a running geomqtt server and renders
the `iss` set fed by `examples/iss-demo`. Designed to be served from
GitHub Pages — see `.github/workflows/pages.yml`.

```
examples/web-iss/
├── index.html        — UI shell, importmap for maplibre-gl
├── src/app.ts        — map + GeomqttSource wiring
├── build.mjs         — esbuild bundle → public/
├── package.json      — file: deps on ../../clients/*
└── README.md
```

## Use the deployed copy

Open the GitHub Pages URL with your server in the query string:

```
https://<org>.github.io/geomqtt/?url=wss://your-geomqtt-host:8083&set=iss
```

The page is served over HTTPS, so the server URL must be `wss://`. Mixed
content (HTTPS page → `ws://` socket) will be blocked by the browser.

## Local dev

Needs Node 20+. Build the clients first, then the demo:

```sh
cd clients && npm install && npm run build
cd ../examples/web-iss && npm install && npm run dev
# open http://localhost:4173/?url=ws://localhost:8083
```

`npm run dev` bundles into `public/` and serves it with `http-server`.
Over plain HTTP the browser allows `ws://`, so `ws://localhost:8083` works.

## URL parameters

| name  | default | meaning                                            |
|-------|---------|----------------------------------------------------|
| `url` | _(prompted)_ | MQTT-over-WebSocket endpoint                   |
| `set` | `iss`   | Redis GEO set name to render                       |

## How it works

`GeomqttSource` (from `@openfantasymap/geomqtt-maplibre`) owns a GeoJSON
source + two circle layers (a halo + a dot). It listens for map
`moveend` / `zoomend` and pushes the current viewport bbox to the
`GeomqttClient`, which subscribes to the covering tiles. Incoming
`add` / `move` / `remove` events feed the GeoJSON source, throttled at
500 ms.

`maplibre-gl` is **not** bundled — it's loaded from jsDelivr via the
importmap in `index.html`. Only the geomqtt client code and `mqtt.js`
land in `public/app.js`.
