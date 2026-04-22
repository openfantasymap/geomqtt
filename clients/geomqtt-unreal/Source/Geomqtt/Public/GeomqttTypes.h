#pragma once

#include "CoreMinimal.h"
#include "GeomqttTypes.generated.h"

USTRUCT(BlueprintType)
struct GEOMQTT_API FGeomqttTileCoord
{
    GENERATED_BODY()

    UPROPERTY(BlueprintReadWrite, Category="geomqtt") int32 Z = 0;
    UPROPERTY(BlueprintReadWrite, Category="geomqtt") int32 X = 0;
    UPROPERTY(BlueprintReadWrite, Category="geomqtt") int32 Y = 0;

    FString Topic(const FString& Set) const
    {
        return FString::Printf(TEXT("geo/%s/%d/%d/%d"), *Set, Z, X, Y);
    }
};

USTRUCT(BlueprintType)
struct GEOMQTT_API FGeomqttBbox
{
    GENERATED_BODY()

    UPROPERTY(BlueprintReadWrite, Category="geomqtt") double West = 0.0;
    UPROPERTY(BlueprintReadWrite, Category="geomqtt") double South = 0.0;
    UPROPERTY(BlueprintReadWrite, Category="geomqtt") double East = 0.0;
    UPROPERTY(BlueprintReadWrite, Category="geomqtt") double North = 0.0;
};

USTRUCT(BlueprintType)
struct GEOMQTT_API FGeomqttFeature
{
    GENERATED_BODY()

    UPROPERTY(BlueprintReadOnly, Category="geomqtt") FString Id;
    UPROPERTY(BlueprintReadOnly, Category="geomqtt") double Lat = 0.0;
    UPROPERTY(BlueprintReadOnly, Category="geomqtt") double Lng = 0.0;

    /** Each value is the JSON-encoded form of the attribute (string values are
     *  bare strings; objects/numbers/bools come through as JSON literals). */
    UPROPERTY(BlueprintReadOnly, Category="geomqtt") TMap<FString, FString> Properties;
};

UENUM(BlueprintType)
enum class EGeomqttFeatureOp : uint8
{
    Snapshot UMETA(DisplayName = "snapshot"),
    Add      UMETA(DisplayName = "add"),
    Move     UMETA(DisplayName = "move"),
    Attr     UMETA(DisplayName = "attr"),
};
