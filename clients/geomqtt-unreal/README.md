# geomqtt for Unreal Engine

Unreal Engine plugin (UE 5.3+) for the
[geomqtt](https://github.com/openfantasymap/geomqtt) server. Subscribes to
tile-keyed MQTT topics that cover an area of interest in your scene, then
raises Blueprint / C++ events as objects are added, moved, and removed.

The MQTT v3.1.1 codec is built in (QoS 0, clean session) and runs over
Unreal's stock `WebSockets` module — **no third-party MQTT plugin
required**.

## Install

1. Drop this folder into your project's `Plugins/Geomqtt/` directory.
2. Regenerate project files and rebuild.
3. The plugin enables itself by default. If you need to flip it manually:
   *Edit → Plugins → Networking → geomqtt*.

## Usage

Two pieces:

* **`UGeomqttClient`** — pure protocol object. Connects, subscribes to
  tiles for an area of interest, raises `OnFeatureUpsert` /
  `OnFeatureRemove` / `OnObjectMessage` delegates on the game thread.
  Usable from C++ or Blueprints.
* **`AGeomqttMarkerSpawner`** — drag-and-drop Actor that wraps a client.
  Set its origin lat/lng at Unreal world `(0, 0, 0)`, give it a marker
  Actor class, and it spawns / moves / destroys per-feature Actors as
  events flow in.

### Quick start (Blueprint, world-anchored)

1. Drop an `AGeomqttMarkerSpawner` into your level.
2. Set:
   * **Url** — `ws://your-server:8083`
   * **Set** — your Redis GEO set name (e.g. `vehicles`)
   * **OriginLat / OriginLon** — coordinates that map to world `(0, 0, 0)`
   * **MarkerClass** — any Actor class to use as a marker (a static mesh
     pawn, a spawn point, anything visible)
3. Press Play. The plugin connects, picks the tiles inside `RadiusMeters`
   of the actor, and spawns markers as features arrive.

### Quick start (C++, custom subscription)

```cpp
#include "GeomqttClient.h"
#include "GeomqttTypes.h"

UGeomqttClient* Client = NewObject<UGeomqttClient>(this);
Client->Url = TEXT("ws://localhost:8083");
Client->OnFeatureUpsert.AddDynamic(this, &AYourActor::HandleUpsert);
Client->Connect();

FGeomqttBbox Bbox{ /*W*/ 11.30, /*S*/ 44.45, /*E*/ 11.45, /*N*/ 44.55 };
Client->SetViewport(TEXT("vehicles"), 14.0, Bbox);
```

### 2D HUD overlay use

Skip `AGeomqttMarkerSpawner`. Instantiate `UGeomqttClient` from a Widget
Blueprint or HUD subclass, listen on `OnFeatureUpsert`, and project the
feature's lat/lng into screen space yourself (or use the included
`UGeomqttTileMath::LatLngToEnu` for a flat-earth metric projection).

## Protocol version

This plugin implements [PROTOCOL.md](../../PROTOCOL.md) v0.1 of the geomqtt
wire contract. The tile-side `attr` op is tolerated as a partial update;
the server v0.1 only emits attribute changes on `objects/<obid>` topics.

## Module layout

```
Source/Geomqtt/
├── Public/
│   ├── Geomqtt.h               — module entry
│   ├── GeomqttTypes.h          — FGeomqttFeature / Bbox / TileCoord, EGeomqttFeatureOp
│   ├── GeomqttTileMath.h       — UBlueprintFunctionLibrary: tile math + ENU
│   ├── GeomqttClient.h         — UGeomqttClient: MQTT-over-WS, subscriptions, state
│   ├── GeomqttSubsystem.h      — UGeomqttSubsystem (UWorldSubsystem)
│   └── GeomqttMarkerSpawner.h  — AGeomqttMarkerSpawner (3D world-anchored driver)
└── Private/
    ├── Geomqtt.cpp             — IMPLEMENT_MODULE
    ├── MqttCodec.h/.cpp        — minimal MQTT v3.1.1 framing
    ├── GeomqttTileMath.cpp
    ├── GeomqttClient.cpp
    ├── GeomqttSubsystem.cpp
    └── GeomqttMarkerSpawner.cpp
```
