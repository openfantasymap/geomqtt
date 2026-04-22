# geomqtt for Unity

Unity client for a [geomqtt](../../) server. Subscribes to tile topics that
cover a configurable area of interest and raises events as objects are added,
moved, and removed.

## Installation

This package depends on two libraries Unity doesn't ship:

1. **Newtonsoft.Json** — declared as a UPM dependency
   (`com.unity.nuget.newtonsoft-json`), installed automatically.
2. **MQTTnet** (v4.x) — install the DLL into `Assets/Plugins/` or reference it
   via any .NET/NuGet resolver for Unity. The package's asmdef uses
   `autoReferenced: true` so MQTTnet is resolved from the project-level
   references. See the MQTTnet releases on GitHub for the correct DLLs for
   your scripting backend (Mono / IL2CPP).

## Usage

Two pieces:

* **`Geomqtt.GeomqttClient`** — pure C# client. Thread-safe for configuration;
  events are queued and drained by calling `PumpEvents()` from the main
  thread (the provided MonoBehaviour does this for you).
* **`Geomqtt.GeomqttWorld3D`** — `MonoBehaviour` that wires the client to a
  Unity scene: origin lat/lng at world `(0,0,0)`, a radius-of-interest in
  meters, and marker GameObjects spawned/moved/destroyed as features arrive.

See `Samples~/BasicMarkers` for a minimal scene setup.

### 2D overlay?

If you're building a flat map UI inside Unity rather than a 3D world, use
`GeomqttClient` directly without `GeomqttWorld3D`. Subscribe to a bbox via
`SetViewportAsync(set, zoom, bbox)` and read `Snapshot()` / listen to
`OnFeatureUpsert` / `OnFeatureRemove` to drive your own rendering
(sprites, UI Toolkit, RectTransforms, whatever fits your project).

## Protocol version

This package implements [PROTOCOL.md](../../PROTOCOL.md) v0.1 of the geomqtt
wire contract. The tile-side `attr` operation is tolerated but not yet
emitted by the server (v0.2).
