#include "GeomqttTileMath.h"

namespace
{
    constexpr double MaxLat = 85.05112878;
    constexpr double EarthR = 6371000.0;
}

FGeomqttTileCoord UGeomqttTileMath::TileForCoord(int32 Z, double Lat, double Lon)
{
    const double N = FMath::Pow(2.0, static_cast<double>(Z));
    const double Clamped = FMath::Clamp(Lat, -MaxLat, MaxLat);
    const double LatRad = FMath::DegreesToRadians(Clamped);
    const double XRaw = (Lon + 180.0) / 360.0 * N;
    const double YRaw =
        (1.0 - FMath::Loge(FMath::Tan(LatRad) + 1.0 / FMath::Cos(LatRad)) / PI) / 2.0 * N;
    const int32 Max = static_cast<int32>(N) - 1;
    FGeomqttTileCoord Out;
    Out.Z = Z;
    Out.X = FMath::Clamp(static_cast<int32>(FMath::FloorToDouble(XRaw)), 0, Max);
    Out.Y = FMath::Clamp(static_cast<int32>(FMath::FloorToDouble(YRaw)), 0, Max);
    return Out;
}

FGeomqttBbox UGeomqttTileMath::BboxForTile(int32 Z, int32 X, int32 Y)
{
    const double N = FMath::Pow(2.0, static_cast<double>(Z));
    FGeomqttBbox B;
    B.West  = static_cast<double>(X) / N * 360.0 - 180.0;
    B.East  = static_cast<double>(X + 1) / N * 360.0 - 180.0;
    B.North = FMath::RadiansToDegrees(FMath::Atan(FMath::Sinh(PI * (1.0 - 2.0 * Y / N))));
    B.South = FMath::RadiansToDegrees(FMath::Atan(FMath::Sinh(PI * (1.0 - 2.0 * (Y + 1) / N))));
    return B;
}

TArray<FGeomqttTileCoord> UGeomqttTileMath::TilesCoveringBbox(int32 Z, const FGeomqttBbox& Bbox)
{
    const double TileSpan = 360.0 / FMath::Pow(2.0, static_cast<double>(Z));
    const double Eps = TileSpan * 1e-9;
    const FGeomqttTileCoord TL = TileForCoord(Z, Bbox.North - Eps, Bbox.West + Eps);
    const FGeomqttTileCoord BR = TileForCoord(Z, Bbox.South + Eps, Bbox.East - Eps);
    const int32 XMin = FMath::Min(TL.X, BR.X);
    const int32 XMax = FMath::Max(TL.X, BR.X);
    const int32 YMin = FMath::Min(TL.Y, BR.Y);
    const int32 YMax = FMath::Max(TL.Y, BR.Y);
    TArray<FGeomqttTileCoord> Out;
    Out.Reserve((XMax - XMin + 1) * (YMax - YMin + 1));
    for (int32 Y = YMin; Y <= YMax; ++Y)
    {
        for (int32 X = XMin; X <= XMax; ++X)
        {
            FGeomqttTileCoord T;
            T.Z = Z;
            T.X = X;
            T.Y = Y;
            Out.Add(T);
        }
    }
    return Out;
}

int32 UGeomqttTileMath::ClosestPublishedZoom(double CurrentZoom, const TArray<int32>& Published)
{
    if (Published.Num() == 0)
    {
        return FMath::FloorToInt32(CurrentZoom);
    }
    TArray<int32> Sorted = Published;
    Sorted.Sort();
    if (CurrentZoom <= Sorted[0])
    {
        return Sorted[0];
    }
    if (CurrentZoom >= Sorted.Last())
    {
        return Sorted.Last();
    }
    int32 Chosen = Sorted[0];
    for (int32 Z : Sorted)
    {
        if (static_cast<double>(Z) <= CurrentZoom)
        {
            Chosen = Z;
        }
        else
        {
            break;
        }
    }
    return Chosen;
}

FVector2D UGeomqttTileMath::LatLngToEnu(double OriginLat, double OriginLon, double Lat, double Lon)
{
    const double DLat = FMath::DegreesToRadians(Lat - OriginLat);
    const double DLon = FMath::DegreesToRadians(Lon - OriginLon);
    const double East  = EarthR * DLon * FMath::Cos(FMath::DegreesToRadians(OriginLat));
    const double North = EarthR * DLat;
    return FVector2D(East, North);
}
