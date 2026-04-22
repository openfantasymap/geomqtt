#pragma once

#include "CoreMinimal.h"
#include "Subsystems/WorldSubsystem.h"
#include "GeomqttSubsystem.generated.h"

class UGeomqttClient;

/**
 * World-scoped subsystem that owns a single shared <see cref="UGeomqttClient"/>.
 * Configure via project settings or grab the client and call Connect() yourself.
 * From Blueprints: `Get Geomqtt Subsystem` → `Get Client`.
 */
UCLASS()
class GEOMQTT_API UGeomqttSubsystem : public UWorldSubsystem
{
    GENERATED_BODY()
public:
    virtual void Initialize(FSubsystemCollectionBase& Collection) override;
    virtual void Deinitialize() override;

    UFUNCTION(BlueprintCallable, Category="geomqtt")
    UGeomqttClient* GetClient() const { return Client; }

private:
    UPROPERTY() UGeomqttClient* Client = nullptr;
};
