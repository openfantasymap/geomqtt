using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using MQTTnet;
using MQTTnet.Client;
using MQTTnet.Protocol;
using Newtonsoft.Json.Linq;

namespace Geomqtt
{
    public class GeomqttOptions
    {
        /// <summary>e.g. "ws://localhost:8083" or "tcp://localhost:1883".</summary>
        public string Url { get; set; } = "tcp://localhost:1883";
        /// <summary>Effective zoom levels the server publishes at. Default matches
        /// GEOMQTT_ENRICH_ZOOMS=6-12 at tile_size=256. If the server is configured
        /// with a different tile_size, seed this from <see cref="ServerConfigFetcher.FetchAsync"/>.</summary>
        public int[] PublishedZooms { get; set; } = new[] { 6, 7, 8, 9, 10, 11, 12 };
        public string? ClientId { get; set; }
        public string? Username { get; set; }
        public string? Password { get; set; }
    }

    /// <summary>Shape of <c>GET /config</c> on the geomqtt server's HTTP port.</summary>
    public class ServerConfig
    {
        public int tileSize { get; set; }
        public int[] zooms { get; set; } = Array.Empty<int>();
        public int[] rawZooms { get; set; } = Array.Empty<int>();
        public string[] enrichAttrs { get; set; } = Array.Empty<string>();
        public string objectKeyPrefix { get; set; } = "";
    }

    public static class ServerConfigFetcher
    {
        static readonly System.Net.Http.HttpClient _http = new();

        /// <summary>Fetch <c>GET /config</c>. Pass the HTTP base URL, e.g. <c>"http://localhost:8080"</c>.</summary>
        public static async Task<ServerConfig> FetchAsync(string httpBaseUrl, CancellationToken ct = default)
        {
            var url = httpBaseUrl.TrimEnd('/') + "/config";
            var json = await _http.GetStringAsync(url, ct);
            return Newtonsoft.Json.JsonConvert.DeserializeObject<ServerConfig>(json)
                ?? throw new InvalidOperationException("geomqtt /config returned null");
        }
    }

    /// <summary>
    /// Protocol-level client. Thread-safe for configuration calls; events are
    /// queued and drained from the main thread via <see cref="PumpEvents"/>.
    /// Call <see cref="PumpEvents"/> from MonoBehaviour.Update — the MQTT
    /// callback runs on a worker thread, so GameObject mutations must be
    /// deferred until the main thread picks them up here.
    /// </summary>
    public class GeomqttClient : IDisposable
    {
        readonly GeomqttOptions _opts;
        readonly IMqttClient _mqtt;
        readonly Dictionary<string, Feature> _features = new();
        readonly HashSet<string> _tileSubs = new();
        readonly HashSet<string> _objectSubs = new();
        readonly ConcurrentQueue<Action> _main = new();
        readonly object _lock = new();

        public event Action? OnConnected;
        public event Action<string?>? OnDisconnected;
        public event Action<Feature, FeatureOp>? OnFeatureUpsert;
        public event Action<string>? OnFeatureRemove;
        public event Action<string, JObject>? OnObjectMessage;
        public event Action<Exception>? OnError;

        public GeomqttClient(GeomqttOptions opts)
        {
            _opts = opts;
            _mqtt = new MqttFactory().CreateMqttClient();
            _mqtt.ApplicationMessageReceivedAsync += HandleMessage;
            _mqtt.ConnectedAsync += _ => { Defer(() => OnConnected?.Invoke()); return Task.CompletedTask; };
            _mqtt.DisconnectedAsync += e => { Defer(() => OnDisconnected?.Invoke(e.Reason.ToString())); return Task.CompletedTask; };
        }

        public async Task ConnectAsync(CancellationToken ct = default)
        {
            var builder = new MqttClientOptionsBuilder()
                .WithProtocolVersion(MQTTnet.Formatter.MqttProtocolVersion.V311)
                .WithCleanSession(true);
            if (_opts.Url.StartsWith("ws", StringComparison.OrdinalIgnoreCase))
                builder = builder.WithWebSocketServer(o => o.WithUri(_opts.Url));
            else
            {
                var uri = new Uri(_opts.Url);
                builder = builder.WithTcpServer(uri.Host, uri.Port > 0 ? uri.Port : 1883);
            }
            if (!string.IsNullOrEmpty(_opts.ClientId)) builder = builder.WithClientId(_opts.ClientId);
            if (!string.IsNullOrEmpty(_opts.Username))
                builder = builder.WithCredentials(_opts.Username, _opts.Password);
            await _mqtt.ConnectAsync(builder.Build(), ct);
        }

        public async Task DisconnectAsync()
        {
            if (_mqtt.IsConnected) await _mqtt.DisconnectAsync();
            lock (_lock)
            {
                _features.Clear();
                _tileSubs.Clear();
                _objectSubs.Clear();
            }
        }

        public void Dispose()
        {
            try { _mqtt.Dispose(); } catch { }
        }

        /// <summary>Call from the main thread each frame to deliver queued events.</summary>
        public void PumpEvents(int budget = 256)
        {
            while (budget-- > 0 && _main.TryDequeue(out var act))
            {
                try { act(); }
                catch (Exception e) { OnError?.Invoke(e); }
            }
        }

        public IReadOnlyCollection<Feature> Snapshot()
        {
            lock (_lock) return _features.Values.ToList();
        }

        public async Task SetViewportAsync(string set, double currentZoom, Bbox bbox)
        {
            int z = TileMath.ClosestPublishedZoom(currentZoom, _opts.PublishedZooms);
            var tiles = TileMath.TilesCoveringBbox(z, bbox);
            var nextTopics = new HashSet<string>(tiles.Select(t => t.Topic(set)));
            List<string> toSub, toUns;
            lock (_lock)
            {
                (toSub, toUns) = Viewport.Diff(_tileSubs, nextTopics);
                foreach (var t in toUns) _tileSubs.Remove(t);
                foreach (var t in toSub) _tileSubs.Add(t);
                EvictFeaturesOutsideTopics(nextTopics, set, z);
            }
            if (toUns.Count > 0)
                await _mqtt.UnsubscribeAsync(toUns.ToArray());
            if (toSub.Count > 0)
            {
                var opts = new MqttClientSubscribeOptionsBuilder();
                foreach (var t in toSub)
                    opts = opts.WithTopicFilter(f => f.WithTopic(t).WithQualityOfServiceLevel(MqttQualityOfServiceLevel.AtMostOnce));
                await _mqtt.SubscribeAsync(opts.Build());
            }
        }

        public async Task SubscribeObjectAsync(string obid)
        {
            var topic = $"objects/{obid}";
            lock (_lock) { if (!_objectSubs.Add(topic)) return; }
            await _mqtt.SubscribeAsync(topic, MqttQualityOfServiceLevel.AtMostOnce);
        }

        public async Task UnsubscribeObjectAsync(string obid)
        {
            var topic = $"objects/{obid}";
            lock (_lock) { if (!_objectSubs.Remove(topic)) return; }
            await _mqtt.UnsubscribeAsync(topic);
        }

        Task HandleMessage(MqttApplicationMessageReceivedEventArgs e)
        {
            try
            {
                var topic = e.ApplicationMessage.Topic;
                var payloadBytes = e.ApplicationMessage.PayloadSegment.Array ?? Array.Empty<byte>();
                var offset = e.ApplicationMessage.PayloadSegment.Offset;
                var count = e.ApplicationMessage.PayloadSegment.Count;
                var json = Encoding.UTF8.GetString(payloadBytes, offset, count);
                var obj = JObject.Parse(json);
                if (topic.StartsWith("geo/")) DispatchTile(obj);
                else if (topic.StartsWith("objects/"))
                    DispatchObject(topic.Substring("objects/".Length), obj);
            }
            catch (Exception ex) { Defer(() => OnError?.Invoke(ex)); }
            return Task.CompletedTask;
        }

        void DispatchTile(JObject obj)
        {
            var op = obj.Value<string>("op");
            var id = obj.Value<string>("id");
            if (string.IsNullOrEmpty(id)) return;
            switch (op)
            {
                case "snapshot":
                case "add":
                {
                    var feat = UpsertFeature(id, obj["lat"]!.Value<double>(), obj["lng"]!.Value<double>(),
                        (obj["attrs"] as JObject)?.Properties().ToDictionary(p => p.Name, p => p.Value));
                    var kind = op == "snapshot" ? FeatureOp.Snapshot : FeatureOp.Add;
                    Defer(() => OnFeatureUpsert?.Invoke(feat, kind));
                    break;
                }
                case "move":
                {
                    var feat = UpsertFeature(id, obj["lat"]!.Value<double>(), obj["lng"]!.Value<double>(), null);
                    Defer(() => OnFeatureUpsert?.Invoke(feat, FeatureOp.Move));
                    break;
                }
                case "remove":
                {
                    bool removed;
                    lock (_lock) removed = _features.Remove(id);
                    if (removed) Defer(() => OnFeatureRemove?.Invoke(id));
                    break;
                }
                case "attr":
                {
                    var attrs = (obj["attrs"] as JObject)?.Properties().ToDictionary(p => p.Name, p => p.Value);
                    Feature? feat;
                    lock (_lock)
                    {
                        if (!_features.TryGetValue(id, out feat)) return;
                        if (attrs != null) foreach (var kv in attrs) feat.Properties[kv.Key] = kv.Value;
                    }
                    Defer(() => OnFeatureUpsert?.Invoke(feat!, FeatureOp.Attr));
                    break;
                }
            }
        }

        void DispatchObject(string id, JObject obj)
        {
            Defer(() => OnObjectMessage?.Invoke(id, obj));
            // Also merge attrs into any tracked feature.
            var op = obj.Value<string>("op");
            if (op is "snapshot" or "attr")
            {
                var attrs = (obj["attrs"] as JObject)?.Properties().ToDictionary(p => p.Name, p => p.Value);
                if (attrs == null) return;
                Feature? feat;
                lock (_lock)
                {
                    if (!_features.TryGetValue(id, out feat)) return;
                    foreach (var kv in attrs) feat.Properties[kv.Key] = kv.Value;
                }
                Defer(() => OnFeatureUpsert?.Invoke(feat!, FeatureOp.Attr));
            }
        }

        Feature UpsertFeature(string id, double lat, double lng, Dictionary<string, JToken>? attrs)
        {
            lock (_lock)
            {
                if (!_features.TryGetValue(id, out var feat))
                {
                    feat = new Feature { Id = id };
                    _features[id] = feat;
                }
                feat.Lat = lat;
                feat.Lng = lng;
                if (attrs != null)
                    foreach (var kv in attrs) feat.Properties[kv.Key] = kv.Value;
                return feat;
            }
        }

        void Defer(Action a) => _main.Enqueue(a);

        void EvictFeaturesOutsideTopics(HashSet<string> nextTopics, string set, int z)
        {
            var toDrop = new List<string>();
            foreach (var (id, feat) in _features)
            {
                var tc = TileMath.TileForCoord(z, feat.Lat, feat.Lng);
                var topic = tc.Topic(set);
                if (!nextTopics.Contains(topic) && !_objectSubs.Contains($"objects/{id}"))
                    toDrop.Add(id);
            }
            foreach (var id in toDrop)
            {
                _features.Remove(id);
                var captured = id;
                Defer(() => OnFeatureRemove?.Invoke(captured));
            }
        }
    }
}
