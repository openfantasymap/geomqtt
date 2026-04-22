using System;
using System.Collections.Generic;

namespace Geomqtt
{
    /// <summary>Slippy-map XYZ tile math (Web Mercator). Mirrors the server `coord.rs`.</summary>
    public static class TileMath
    {
        const double MaxLat = 85.05112878;

        public static TileCoord TileForCoord(int z, double lat, double lon)
        {
            double n = Math.Pow(2, z);
            double clamped = Math.Max(-MaxLat, Math.Min(MaxLat, lat));
            double latRad = clamped * Math.PI / 180.0;
            int x = (int)Math.Floor((lon + 180.0) / 360.0 * n);
            int y = (int)Math.Floor((1.0 - Math.Log(Math.Tan(latRad) + 1.0 / Math.Cos(latRad)) / Math.PI) / 2.0 * n);
            int max = (int)n - 1;
            return new TileCoord(z, Math.Clamp(x, 0, max), Math.Clamp(y, 0, max));
        }

        public static Bbox BboxForTile(int z, int x, int y)
        {
            double n = Math.Pow(2, z);
            double w = x / n * 360.0 - 180.0;
            double e = (x + 1) / n * 360.0 - 180.0;
            double north = Math.Atan(Math.Sinh(Math.PI * (1.0 - 2.0 * y / n))) * 180.0 / Math.PI;
            double south = Math.Atan(Math.Sinh(Math.PI * (1.0 - 2.0 * (y + 1) / n))) * 180.0 / Math.PI;
            return new Bbox { West = w, South = south, East = e, North = north };
        }

        public static List<TileCoord> TilesCoveringBbox(int z, Bbox b)
        {
            var tl = TileForCoord(z, b.North, b.West);
            var br = TileForCoord(z, b.South, b.East);
            int xMin = Math.Min(tl.X, br.X);
            int xMax = Math.Max(tl.X, br.X);
            int yMin = Math.Min(tl.Y, br.Y);
            int yMax = Math.Max(tl.Y, br.Y);
            var list = new List<TileCoord>((xMax - xMin + 1) * (yMax - yMin + 1));
            for (int y = yMin; y <= yMax; y++)
                for (int x = xMin; x <= xMax; x++)
                    list.Add(new TileCoord(z, x, y));
            return list;
        }

        /// <summary>Largest published zoom ≤ current, clamped to the published range.</summary>
        public static int ClosestPublishedZoom(double current, IReadOnlyList<int> published)
        {
            if (published == null || published.Count == 0) return (int)Math.Floor(current);
            var sorted = new List<int>(published);
            sorted.Sort();
            if (current <= sorted[0]) return sorted[0];
            if (current >= sorted[^1]) return sorted[^1];
            int chosen = sorted[0];
            foreach (var z in sorted)
            {
                if (z <= current) chosen = z;
                else break;
            }
            return chosen;
        }
    }

    /// <summary>Flat-earth ENU projection. Accurate for game worlds ≤ a few km.</summary>
    public static class Geodesy
    {
        const double EarthR = 6_371_000.0;

        /// <summary>Returns meters east, meters north of the origin.</summary>
        public static (double east, double north) ToEnu(double originLat, double originLon, double lat, double lon)
        {
            double dLat = (lat - originLat) * Math.PI / 180.0;
            double dLon = (lon - originLon) * Math.PI / 180.0;
            double east = EarthR * dLon * Math.Cos(originLat * Math.PI / 180.0);
            double north = EarthR * dLat;
            return (east, north);
        }

        public static (double lat, double lon) FromEnu(double originLat, double originLon, double east, double north)
        {
            double lat = originLat + (north / EarthR) * 180.0 / Math.PI;
            double lon = originLon + (east / EarthR) * 180.0 / Math.PI / Math.Cos(originLat * Math.PI / 180.0);
            return (lat, lon);
        }
    }
}
