#pragma once

#include "CoreMinimal.h"
#include "UObject/Object.h"
#include "Containers/Ticker.h"
#include "GeomqttTypes.h"
#include "GeomqttClient.generated.h"

class IWebSocket;

DECLARE_DYNAMIC_MULTICAST_DELEGATE(FGeomqttOnConnected);
DECLARE_DYNAMIC_MULTICAST_DELEGATE_OneParam(FGeomqttOnDisconnected, FString, Reason);
DECLARE_DYNAMIC_MULTICAST_DELEGATE_TwoParams(FGeomqttOnFeatureUpsert,
    const FGeomqttFeature&, Feature, EGeomqttFeatureOp, Op);
DECLARE_DYNAMIC_MULTICAST_DELEGATE_OneParam(FGeomqttOnFeatureRemove, FString, Id);
DECLARE_DYNAMIC_MULTICAST_DELEGATE_TwoParams(FGeomqttOnObjectMessage,
    FString, Id, FString, JsonText);

/**
 * Protocol-level client. Talks MQTT v3.1.1 (QoS 0, clean session) to a
 * geomqtt server's WebSocket endpoint. Maintains an id-keyed feature state
 * and broadcasts upsert / remove events. Safe to use from Blueprints — all
 * delegate callbacks fire on the game thread.
 */
UCLASS(BlueprintType, Blueprintable)
class GEOMQTT_API UGeomqttClient : public UObject
{
    GENERATED_BODY()
public:
    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="geomqtt")
    FString Url = TEXT("ws://localhost:8083");

    /** Effective zoom levels the server publishes at. Match what `GET /config`
     *  returns; default tracks GEOMQTT_ENRICH_ZOOMS=6-12 at tile_size=256. */
    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="geomqtt")
    TArray<int32> PublishedZooms = { 6, 7, 8, 9, 10, 11, 12 };

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="geomqtt")
    FString ClientId;

    UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="geomqtt")
    int32 KeepAliveSeconds = 30;

    UPROPERTY(BlueprintAssignable, Category="geomqtt") FGeomqttOnConnected      OnConnected;
    UPROPERTY(BlueprintAssignable, Category="geomqtt") FGeomqttOnDisconnected   OnDisconnected;
    UPROPERTY(BlueprintAssignable, Category="geomqtt") FGeomqttOnFeatureUpsert  OnFeatureUpsert;
    UPROPERTY(BlueprintAssignable, Category="geomqtt") FGeomqttOnFeatureRemove  OnFeatureRemove;
    UPROPERTY(BlueprintAssignable, Category="geomqtt") FGeomqttOnObjectMessage  OnObjectMessage;

    UFUNCTION(BlueprintCallable, Category="geomqtt") void Connect();
    UFUNCTION(BlueprintCallable, Category="geomqtt") void Disconnect();

    /** Update the area of interest. Picks the closest published zoom, computes
     *  tile coverage, diffs against the previous subscription set, sends
     *  SUBSCRIBE / UNSUBSCRIBE for the deltas. */
    UFUNCTION(BlueprintCallable, Category="geomqtt")
    void SetViewport(const FString& Set, double CurrentZoom, const FGeomqttBbox& Bbox);

    UFUNCTION(BlueprintCallable, Category="geomqtt")
    void SubscribeObject(const FString& ObId);

    UFUNCTION(BlueprintCallable, Category="geomqtt")
    void UnsubscribeObject(const FString& ObId);

    UFUNCTION(BlueprintPure, Category="geomqtt")
    TArray<FGeomqttFeature> GetSnapshot() const;

    virtual void BeginDestroy() override;

private:
    TSharedPtr<IWebSocket> WebSocket;
    TArray<uint8> InboundBuffer;
    TMap<FString, FGeomqttFeature> Features;
    TSet<FString> TileSubs;
    TSet<FString> ObjectSubs;
    uint16 NextPacketId = 1;
    FTSTicker::FDelegateHandle KeepaliveHandle;
    bool bMqttConnected = false;
    FString LastSet;
    int32 LastZoom = 0;

    void HandleConnected();
    void HandleClosed(int32 StatusCode, const FString& Reason, bool bWasClean);
    void HandleConnectionError(const FString& Error);
    void HandleRawMessage(const void* Data, SIZE_T Size, SIZE_T BytesRemaining);
    void DrainInbound();
    void OnPublish(const FString& Topic, const TArray<uint8>& Payload);
    void DispatchTile(const FString& Topic, const TSharedPtr<class FJsonObject>& Json);
    void DispatchObject(const FString& ObId, const FString& JsonText, const TSharedPtr<class FJsonObject>& Json);
    void EvictFeaturesOutsideTopics(const TSet<FString>& NextTopics, const FString& Set, int32 Z);

    void SendBytes(const TArray<uint8>& Bytes);
    bool TickKeepalive(float DeltaTime);
    static FGeomqttFeature MakeOrMergeFeature(TMap<FString, FGeomqttFeature>& Features,
                                              const FString& Id, double Lat, double Lng,
                                              const TSharedPtr<class FJsonObject>& Attrs);
    static void MergeAttrsInto(FGeomqttFeature& Feat, const TSharedPtr<class FJsonObject>& Attrs);
    static FString TileTopic(const FString& Set, int32 Z, int32 X, int32 Y);
};
