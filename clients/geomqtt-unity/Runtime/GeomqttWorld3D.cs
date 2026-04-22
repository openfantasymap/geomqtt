using System.Collections.Generic;
using UnityEngine;

namespace Geomqtt
{
    /// <summary>
    /// 3D world-anchored driver. Attach to any GameObject; set the origin
    /// lat/lng and assign a marker prefab. On Start(), connects and subscribes
    /// to tiles within <see cref="RadiusMeters"/> of <see cref="AnchorTransform"/>
    /// (or the GameObject's position if none is set). Spawns/moves/destroys
    /// marker GameObjects as features arrive.
    /// </summary>
    public class GeomqttWorld3D : MonoBehaviour
    {
        [Header("Server")]
        public string Url = "ws://localhost:8083";
        public string Set = "vehicles";
        public int[] PublishedZooms = new[] { 4, 8, 12, 16 };

        [Header("Anchor")]
        [Tooltip("Lat/Lng of Unity world origin (0,0,0).")]
        public double OriginLat = 44.49;
        public double OriginLon = 11.34;
        [Tooltip("If null, this component's transform is used as the area-of-interest center.")]
        public Transform? AnchorTransform;
        [Tooltip("Radius of the circular AoI used to compute the viewport bbox (meters).")]
        public float RadiusMeters = 500f;
        [Tooltip("Nominal zoom level used when picking published-zoom tiles.")]
        public float ZoomLevel = 14f;
        [Tooltip("Viewport update throttle (seconds).")]
        public float ViewportUpdateInterval = 0.25f;

        [Header("Rendering")]
        public GameObject? MarkerPrefab;
        public float MarkerHeightY = 1f;

        GeomqttClient? _client;
        readonly Dictionary<string, GameObject> _markers = new();
        float _nextViewportTime;

        void Start()
        {
            _client = new GeomqttClient(new GeomqttOptions { Url = Url, PublishedZooms = PublishedZooms });
            _client.OnFeatureUpsert += UpsertMarker;
            _client.OnFeatureRemove += RemoveMarker;
            _client.OnError += ex => Debug.LogError($"[geomqtt] {ex}");
            _ = _client.ConnectAsync();
        }

        void Update()
        {
            _client?.PumpEvents();
            if (Time.time >= _nextViewportTime)
            {
                _nextViewportTime = Time.time + ViewportUpdateInterval;
                _ = PushViewport();
            }
        }

        async System.Threading.Tasks.Task PushViewport()
        {
            if (_client == null) return;
            var anchor = AnchorTransform != null ? AnchorTransform.position : transform.position;
            var (lat, lon) = Geodesy.FromEnu(OriginLat, OriginLon, anchor.x, anchor.z);
            // Expand RadiusMeters in each cardinal direction.
            var (eastLat, eastLon) = Geodesy.FromEnu(lat, lon, RadiusMeters, 0);
            var (_, westLon) = Geodesy.FromEnu(lat, lon, -RadiusMeters, 0);
            var (_, _) = (eastLat, eastLon); // silence unused
            var (northLat, _) = Geodesy.FromEnu(lat, lon, 0, RadiusMeters);
            var (southLat, _) = Geodesy.FromEnu(lat, lon, 0, -RadiusMeters);
            var bbox = new Bbox { West = westLon, South = southLat, East = eastLon, North = northLat };
            await _client.SetViewportAsync(Set, ZoomLevel, bbox);
        }

        void OnDestroy()
        {
            if (_client != null)
            {
                _client.OnFeatureUpsert -= UpsertMarker;
                _client.OnFeatureRemove -= RemoveMarker;
                _ = _client.DisconnectAsync();
                _client.Dispose();
            }
            foreach (var go in _markers.Values) if (go != null) Destroy(go);
            _markers.Clear();
        }

        void UpsertMarker(Feature f, FeatureOp op)
        {
            var (east, north) = Geodesy.ToEnu(OriginLat, OriginLon, f.Lat, f.Lng);
            var worldPos = new Vector3((float)east, MarkerHeightY, (float)north);
            if (!_markers.TryGetValue(f.Id, out var go) || go == null)
            {
                go = MarkerPrefab != null
                    ? Instantiate(MarkerPrefab, worldPos, Quaternion.identity, transform)
                    : GameObject.CreatePrimitive(PrimitiveType.Sphere);
                go.transform.position = worldPos;
                go.name = $"geomqtt:{f.Id}";
                _markers[f.Id] = go;
            }
            else
            {
                go.transform.position = worldPos;
            }
        }

        void RemoveMarker(string id)
        {
            if (_markers.TryGetValue(id, out var go))
            {
                if (go != null) Destroy(go);
                _markers.Remove(id);
            }
        }
    }
}
