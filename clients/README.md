# geomqtt clients

Client libraries that consume the [geomqtt](../) server's MQTT topic tree.

## JavaScript / TypeScript workspace

An npm workspace with three packages:

* **`@openfantasymap/geomqtt-core`** — protocol types, tile math, MQTT
  transport, and the viewport → subscription diffing loop. No DOM
  dependency; works in Node and the browser. Built on top of
  [`mqtt.js`](https://github.com/mqttjs/MQTT.js).
* **`@openfantasymap/geomqtt-leaflet`** — `L.LayerGroup` adapter around
  core. Wires `moveend` / `zoomend` to viewport updates and maintains a
  marker per feature. Defaults to `L.circleMarker`; override via
  `markerFor`.
* **`@openfantasymap/geomqtt-maplibre`** — MapLibre / Mapbox GL adapter.
  Keeps a GeoJSON source fed from the current state and attaches a default
  circle layer (overridable via `layers`).

> **Published to GitHub Packages** at `https://npm.pkg.github.com`. To
> install, add the `@openfantasymap` scope mapping to a `.npmrc`:
>
> ```ini
> @openfantasymap:registry=https://npm.pkg.github.com
> //npm.pkg.github.com/:_authToken=YOUR_GITHUB_PAT
> ```
>
> A GitHub PAT with the `read:packages` scope is enough.

### Build

```sh
cd clients
npm install
npm run build
```

### Usage (Leaflet)

```ts
import L from 'leaflet';
import { GeomqttLayer } from '@openfantasymap/geomqtt-leaflet';

const map = L.map('map').setView([44.49, 11.34], 14);
L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png').addTo(map);

new GeomqttLayer({
  url: 'ws://localhost:8083',
  set: 'vehicles',
}).addTo(map);
```

### Usage (MapLibre)

```ts
import maplibregl from 'maplibre-gl';
import { GeomqttSource } from '@openfantasymap/geomqtt-maplibre';

const map = new maplibregl.Map({
  container: 'map',
  style: 'https://demotiles.maplibre.org/style.json',
  center: [11.34, 44.49],
  zoom: 14,
});
map.on('load', async () => {
  const src = new GeomqttSource({
    map,
    url: 'ws://localhost:8083',
    set: 'vehicles',
  });
  await src.attach();
});
```

## Unity package

`geomqtt-unity/` is a UPM package (`com.geomqtt.unity`) — see its own
[README](./geomqtt-unity/README.md) for installation and usage. It mirrors
the protocol and viewport logic from `@openfantasymap/geomqtt-core` in C#, with a 3D
world-anchored `MonoBehaviour` (`GeomqttWorld3D`) as the default driver and
a note on how to use the plain `GeomqttClient` for 2D overlays.
