#include "GeomqttMarkerSpawner.h"
#include "GeomqttClient.h"
#include "GeomqttTileMath.h"
#include "Engine/World.h"

AGeomqttMarkerSpawner::AGeomqttMarkerSpawner()
{
    PrimaryActorTick.bCanEverTick = true;
}

void AGeomqttMarkerSpawner::BeginPlay()
{
    Super::BeginPlay();
    Client = NewObject<UGeomqttClient>(this);
    Client->Url = Url;
    Client->PublishedZooms = PublishedZooms;
    Client->OnFeatureUpsert.AddDynamic(this, &AGeomqttMarkerSpawner::HandleUpsert);
    Client->OnFeatureRemove.AddDynamic(this, &AGeomqttMarkerSpawner::HandleRemove);
    Client->Connect();
}

void AGeomqttMarkerSpawner::EndPlay(const EEndPlayReason::Type EndPlayReason)
{
    if (Client)
    {
        Client->OnFeatureUpsert.RemoveDynamic(this, &AGeomqttMarkerSpawner::HandleUpsert);
        Client->OnFeatureRemove.RemoveDynamic(this, &AGeomqttMarkerSpawner::HandleRemove);
        Client->Disconnect();
        Client = nullptr;
    }
    for (auto& Pair : Markers)
    {
        if (Pair.Value)
        {
            Pair.Value->Destroy();
        }
    }
    Markers.Reset();
    Super::EndPlay(EndPlayReason);
}

void AGeomqttMarkerSpawner::Tick(float DeltaTime)
{
    Super::Tick(DeltaTime);
    TimeUntilViewportUpdate -= DeltaTime;
    if (TimeUntilViewportUpdate <= 0.f)
    {
        TimeUntilViewportUpdate = ViewportUpdateInterval;
        PushViewport();
    }
}

void AGeomqttMarkerSpawner::PushViewport()
{
    if (!Client) return;
    Client->SetViewport(Set, ZoomLevel, AreaOfInterestBbox());
}

FGeomqttBbox AGeomqttMarkerSpawner::AreaOfInterestBbox() const
{
    // Treat the actor's world position as the centre of the area of interest.
    // Convert centre back to lat/lng, then extend RadiusMeters in each cardinal
    // direction.
    const FVector ActorPos = GetActorLocation();
    const double EastM  = ActorPos.X / UnrealUnitsPerMetre;
    const double NorthM = ActorPos.Y / UnrealUnitsPerMetre;
    constexpr double EarthR = 6371000.0;
    const double CentreLat = OriginLat + (NorthM / EarthR) * 180.0 / PI;
    const double CentreLon = OriginLon + (EastM / EarthR) * 180.0 / PI
        / FMath::Cos(FMath::DegreesToRadians(OriginLat));

    const double DLat = (RadiusMeters / EarthR) * 180.0 / PI;
    const double DLon = (RadiusMeters / EarthR) * 180.0 / PI
        / FMath::Cos(FMath::DegreesToRadians(CentreLat));
    FGeomqttBbox B;
    B.West  = CentreLon - DLon;
    B.East  = CentreLon + DLon;
    B.South = CentreLat - DLat;
    B.North = CentreLat + DLat;
    return B;
}

void AGeomqttMarkerSpawner::HandleUpsert(const FGeomqttFeature& Feature, EGeomqttFeatureOp /*Op*/)
{
    const FVector Pos = LatLngToWorld(Feature.Lat, Feature.Lng);
    AActor** Existing = Markers.Find(Feature.Id);
    if (!Existing || !*Existing)
    {
        AActor* Spawned = nullptr;
        if (MarkerClass)
        {
            FActorSpawnParameters Params;
            Params.SpawnCollisionHandlingOverride =
                ESpawnActorCollisionHandlingMethod::AlwaysSpawn;
            Spawned = GetWorld()->SpawnActor<AActor>(MarkerClass, Pos, FRotator::ZeroRotator, Params);
        }
        if (Spawned)
        {
            Spawned->SetActorLabel(FString::Printf(TEXT("geomqtt:%s"), *Feature.Id));
            Markers.Add(Feature.Id, Spawned);
        }
    }
    else
    {
        (*Existing)->SetActorLocation(Pos);
    }
}

void AGeomqttMarkerSpawner::HandleRemove(FString Id)
{
    AActor** Existing = Markers.Find(Id);
    if (Existing && *Existing)
    {
        (*Existing)->Destroy();
        Markers.Remove(Id);
    }
}

FVector AGeomqttMarkerSpawner::LatLngToWorld(double Lat, double Lng) const
{
    const FVector2D Enu = UGeomqttTileMath::LatLngToEnu(OriginLat, OriginLon, Lat, Lng);
    return FVector(
        static_cast<double>(Enu.X) * UnrealUnitsPerMetre,
        static_cast<double>(Enu.Y) * UnrealUnitsPerMetre,
        static_cast<double>(MarkerHeightZ));
}
