#include "GeomqttSubsystem.h"
#include "GeomqttClient.h"

void UGeomqttSubsystem::Initialize(FSubsystemCollectionBase& Collection)
{
    Super::Initialize(Collection);
    Client = NewObject<UGeomqttClient>(this);
}

void UGeomqttSubsystem::Deinitialize()
{
    if (Client)
    {
        Client->Disconnect();
        Client = nullptr;
    }
    Super::Deinitialize();
}
