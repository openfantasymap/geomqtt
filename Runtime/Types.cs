using System.Collections.Generic;
using Newtonsoft.Json.Linq;

namespace Geomqtt
{
    public struct TileCoord
    {
        public int Z;
        public int X;
        public int Y;

        public TileCoord(int z, int x, int y) { Z = z; X = x; Y = y; }
        public string Topic(string set) => $"geo/{set}/{Z}/{X}/{Y}";
        public override string ToString() => $"({Z}/{X}/{Y})";
    }

    public struct Bbox
    {
        public double West;
        public double South;
        public double East;
        public double North;
    }

    /// <summary>Position + attributes of one tracked object as the client sees it.</summary>
    public class Feature
    {
        public string Id { get; set; } = "";
        public double Lat { get; set; }
        public double Lng { get; set; }
        public Dictionary<string, JToken> Properties { get; set; } = new();
    }

    public enum FeatureOp
    {
        Snapshot,
        Add,
        Move,
        Attr,
    }
}
