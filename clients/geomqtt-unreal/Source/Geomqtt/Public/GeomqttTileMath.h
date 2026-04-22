#pragma once

#include "CoreMinimal.h"
#include "Kismet/BlueprintFunctionLibrary.h"
#include "GeomqttTypes.h"
#include "GeomqttTileMath.generated.h"

/** Slippy-map XYZ tile math + flat-earth ENU projection. Mirrors coord.rs /
 *  coord.ts / TileMath.cs. Pure functions, BlueprintPure where possible. */
UCLASS()
class GEOMQTT_API UGeomqttTileMath : public UBlueprintFunctionLibrary
{
    GENERATED_BODY()
public:
    UFUNCTION(BlueprintPure, Category="geomqtt|tilemath")
    static FGeomqttTileCoord TileForCoord(int32 Z, double Lat, double Lon);

    UFUNCTION(BlueprintPure, Category="geomqtt|tilemath")
    static FGeomqttBbox BboxForTile(int32 Z, int32 X, int32 Y);

    /** Tiles that intersect the bbox at zoom Z. Boundary-tolerant: a bbox
     *  edge that lands exactly on a tile boundary belongs to the inside,
     *  not the neighbour. */
    UFUNCTION(BlueprintPure, Category="geomqtt|tilemath")
    static TArray<FGeomqttTileCoord> TilesCoveringBbox(int32 Z, const FGeomqttBbox& Bbox);

    /** Largest published zoom <= CurrentZoom; clamped to the published range. */
    UFUNCTION(BlueprintPure, Category="geomqtt|tilemath")
    static int32 ClosestPublishedZoom(double CurrentZoom, const TArray<int32>& Published);

    /** Flat-earth projection: lat/lng → meters east, meters north of the
     *  origin. Accurate for game worlds <= a few km. */
    UFUNCTION(BlueprintPure, Category="geomqtt|geodesy")
    static FVector2D LatLngToEnu(double OriginLat, double OriginLon, double Lat, double Lon);
};
