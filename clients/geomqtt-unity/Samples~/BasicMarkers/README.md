# Basic Markers

Drop a `GeomqttWorld3D` component on a GameObject in your scene:

1. Set **Url** to your geomqtt server's WebSocket or TCP endpoint.
2. Set **Set** to the Redis GEO set you want to render (e.g. `vehicles`).
3. Set **OriginLat / OriginLon** to the lat/lng that corresponds to Unity's
   world origin `(0, 0, 0)`.
4. Optionally assign a **MarkerPrefab** — a simple sphere is used when none
   is provided.

On play, the component connects, subscribes to tiles covering a circle of
radius `RadiusMeters` around its transform, and spawns/moves/destroys marker
GameObjects as features arrive from the server.

Use with `GEOADD vehicles <lon> <lat> <obid>` on the upstream Redis (via
`redis-cli -p 6380` against the geomqtt RESP port) to push updates.
