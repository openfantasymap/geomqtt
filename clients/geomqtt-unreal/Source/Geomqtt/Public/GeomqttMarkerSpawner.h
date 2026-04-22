#pragma once

#include "CoreMinimal.h"
#include "GameFramework/Actor.h"
#include "GeomqttTypes.h"
#include "GeomqttMarkerSpawner.generated.h"

class UGeomqttClient;

/**
 * 3D world-anchored driver. Drop into a level, set OriginLat/Lon at world
 * (0,0,0), assign a MarkerClass, and on BeginPlay it connects, subscribes
 * to tiles within RadiusMeters of itself, and spawns/moves/destroys actors
 * as features arrive from the server. Mirrors the Unity GeomqttWorld3D
 * MonoBehaviour.
 */
UCLASS(BlueprintType, Blueprintable, Category="geomqtt")
class GEOMQTT_API AGeomqttMarkerSpawner : public AActor
{
    GENERATED_BODY()
public:
    AGeomqttMarkerSpawner();

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="server")
    FString Url = TEXT("ws://localhost:8083");

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="server")
    FString Set = TEXT("vehicles");

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="server")
    TArray<int32> PublishedZooms = { 6, 7, 8, 9, 10, 11, 12 };

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="anchor")
    double OriginLat = 44.49;

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="anchor")
    double OriginLon = 11.34;

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="anchor")
    float RadiusMeters = 500.f;

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="anchor")
    float ZoomLevel = 14.f;

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="anchor")
    float ViewportUpdateInterval = 0.25f;

    /** Centimetres per metre. Unreal's default unit is cm; keep at 100. */
    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="anchor")
    float UnrealUnitsPerMetre = 100.f;

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="rendering")
    TSubclassOf<AActor> MarkerClass;

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="rendering")
    float MarkerHeightZ = 100.f;

protected:
    virtual void BeginPlay() override;
    virtual void EndPlay(const EEndPlayReason::Type EndPlayReason) override;
    virtual void Tick(float DeltaTime) override;

private:
    UPROPERTY() UGeomqttClient* Client = nullptr;
    UPROPERTY() TMap<FString, AActor*> Markers;
    float TimeUntilViewportUpdate = 0.f;

    UFUNCTION() void HandleUpsert(const FGeomqttFeature& Feature, EGeomqttFeatureOp Op);
    UFUNCTION() void HandleRemove(FString Id);

    void PushViewport();
    FVector LatLngToWorld(double Lat, double Lng) const;
    FGeomqttBbox AreaOfInterestBbox() const;
};
