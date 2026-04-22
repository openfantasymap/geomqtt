#include "GeomqttClient.h"
#include "GeomqttTileMath.h"
#include "MqttCodec.h"

#include "WebSocketsModule.h"
#include "IWebSocket.h"
#include "Dom/JsonObject.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"
#include "Containers/Ticker.h"
#include "Misc/Guid.h"

using namespace Geomqtt;

void UGeomqttClient::Connect()
{
    if (WebSocket.IsValid())
    {
        return;
    }
    if (ClientId.IsEmpty())
    {
        ClientId = FString::Printf(TEXT("geomqtt-ue-%s"),
            *FGuid::NewGuid().ToString(EGuidFormats::DigitsLower).Left(8));
    }

    if (!FModuleManager::Get().IsModuleLoaded("WebSockets"))
    {
        FModuleManager::Get().LoadModule("WebSockets");
    }
    TArray<FString> Protocols = { TEXT("mqtt") };
    WebSocket = FWebSocketsModule::Get().CreateWebSocket(Url, Protocols);

    WebSocket->OnConnected().AddUObject(this, &UGeomqttClient::HandleConnected);
    WebSocket->OnConnectionError().AddUObject(this, &UGeomqttClient::HandleConnectionError);
    WebSocket->OnClosed().AddUObject(this, &UGeomqttClient::HandleClosed);
    WebSocket->OnRawMessage().AddUObject(this, &UGeomqttClient::HandleRawMessage);

    WebSocket->Connect();
}

void UGeomqttClient::Disconnect()
{
    if (KeepaliveHandle.IsValid())
    {
        FTSTicker::GetCoreTicker().RemoveTicker(KeepaliveHandle);
        KeepaliveHandle.Reset();
    }
    if (WebSocket.IsValid())
    {
        if (bMqttConnected)
        {
            TArray<uint8> Bytes;
            Mqtt::EncodeDisconnect(Bytes);
            WebSocket->Send(Bytes.GetData(), Bytes.Num(), /*bIsBinary=*/true);
        }
        WebSocket->Close();
        WebSocket.Reset();
    }
    bMqttConnected = false;
    Features.Reset();
    TileSubs.Reset();
    ObjectSubs.Reset();
    InboundBuffer.Reset();
}

void UGeomqttClient::BeginDestroy()
{
    Disconnect();
    Super::BeginDestroy();
}

void UGeomqttClient::HandleConnected()
{
    TArray<uint8> Bytes;
    Mqtt::EncodeConnect(Bytes, ClientId, static_cast<uint16>(KeepAliveSeconds));
    SendBytes(Bytes);

    KeepaliveHandle = FTSTicker::GetCoreTicker().AddTicker(
        FTickerDelegate::CreateUObject(this, &UGeomqttClient::TickKeepalive),
        FMath::Max(1.f, static_cast<float>(KeepAliveSeconds) * 0.5f));
}

void UGeomqttClient::HandleClosed(int32 StatusCode, const FString& Reason, bool /*bWasClean*/)
{
    bMqttConnected = false;
    OnDisconnected.Broadcast(FString::Printf(TEXT("ws closed (%d): %s"), StatusCode, *Reason));
}

void UGeomqttClient::HandleConnectionError(const FString& Error)
{
    OnDisconnected.Broadcast(FString::Printf(TEXT("ws error: %s"), *Error));
}

void UGeomqttClient::HandleRawMessage(const void* Data, SIZE_T Size, SIZE_T BytesRemaining)
{
    InboundBuffer.Append(static_cast<const uint8*>(Data), Size);
    if (BytesRemaining > 0)
    {
        // The websocket message is fragmented; wait until we have it all.
        return;
    }
    DrainInbound();
}

void UGeomqttClient::DrainInbound()
{
    Mqtt::EPacketType Type;
    Mqtt::FParsedPublish Pub;
    while (Mqtt::TryParseNextPacket(InboundBuffer, Type, Pub))
    {
        switch (Type)
        {
            case Mqtt::EPacketType::ConnAck:
                bMqttConnected = true;
                OnConnected.Broadcast();
                break;
            case Mqtt::EPacketType::Publish:
                OnPublish(Pub.Topic, Pub.Payload);
                break;
            default:
                break; // SUBACK / UNSUBACK / PINGRESP — nothing to do
        }
    }
}

void UGeomqttClient::OnPublish(const FString& Topic, const TArray<uint8>& Payload)
{
    const FString Json = FString(FUTF8ToTCHAR(reinterpret_cast<const ANSICHAR*>(Payload.GetData()), Payload.Num()));
    TSharedPtr<FJsonObject> Obj;
    const TSharedRef<TJsonReader<TCHAR>> Reader = TJsonReaderFactory<TCHAR>::Create(Json);
    if (!FJsonSerializer::Deserialize(Reader, Obj) || !Obj.IsValid())
    {
        return;
    }
    if (Topic.StartsWith(TEXT("geo/")))
    {
        DispatchTile(Topic, Obj);
    }
    else if (Topic.StartsWith(TEXT("objects/")))
    {
        DispatchObject(Topic.RightChop(8), Json, Obj);
    }
}

void UGeomqttClient::DispatchTile(const FString& /*Topic*/, const TSharedPtr<FJsonObject>& Obj)
{
    const FString Op = Obj->GetStringField(TEXT("op"));
    const FString Id = Obj->GetStringField(TEXT("id"));
    if (Id.IsEmpty()) return;

    if (Op == TEXT("snapshot") || Op == TEXT("add"))
    {
        const double Lat = Obj->GetNumberField(TEXT("lat"));
        const double Lng = Obj->GetNumberField(TEXT("lng"));
        const TSharedPtr<FJsonObject>* Attrs = nullptr;
        Obj->TryGetObjectField(TEXT("attrs"), Attrs);
        const FGeomqttFeature Feat = MakeOrMergeFeature(Features, Id, Lat, Lng, Attrs ? *Attrs : nullptr);
        OnFeatureUpsert.Broadcast(Feat,
            Op == TEXT("snapshot") ? EGeomqttFeatureOp::Snapshot : EGeomqttFeatureOp::Add);
    }
    else if (Op == TEXT("move"))
    {
        const double Lat = Obj->GetNumberField(TEXT("lat"));
        const double Lng = Obj->GetNumberField(TEXT("lng"));
        const FGeomqttFeature Feat = MakeOrMergeFeature(Features, Id, Lat, Lng, nullptr);
        OnFeatureUpsert.Broadcast(Feat, EGeomqttFeatureOp::Move);
    }
    else if (Op == TEXT("remove"))
    {
        if (Features.Remove(Id) > 0)
        {
            OnFeatureRemove.Broadcast(Id);
        }
    }
    else if (Op == TEXT("attr"))
    {
        FGeomqttFeature* Existing = Features.Find(Id);
        if (!Existing) return;
        const TSharedPtr<FJsonObject>* Attrs = nullptr;
        if (Obj->TryGetObjectField(TEXT("attrs"), Attrs) && Attrs)
        {
            MergeAttrsInto(*Existing, *Attrs);
        }
        OnFeatureUpsert.Broadcast(*Existing, EGeomqttFeatureOp::Attr);
    }
}

void UGeomqttClient::DispatchObject(const FString& ObId, const FString& JsonText,
                                    const TSharedPtr<FJsonObject>& Obj)
{
    OnObjectMessage.Broadcast(ObId, JsonText);
    const FString Op = Obj->GetStringField(TEXT("op"));
    if (Op == TEXT("snapshot") || Op == TEXT("attr"))
    {
        FGeomqttFeature* Existing = Features.Find(ObId);
        if (!Existing) return;
        const TSharedPtr<FJsonObject>* Attrs = nullptr;
        if (Obj->TryGetObjectField(TEXT("attrs"), Attrs) && Attrs)
        {
            MergeAttrsInto(*Existing, *Attrs);
            OnFeatureUpsert.Broadcast(*Existing, EGeomqttFeatureOp::Attr);
        }
    }
}

FGeomqttFeature UGeomqttClient::MakeOrMergeFeature(TMap<FString, FGeomqttFeature>& Features,
                                                   const FString& Id, double Lat, double Lng,
                                                   const TSharedPtr<FJsonObject>& Attrs)
{
    FGeomqttFeature& Feat = Features.FindOrAdd(Id);
    Feat.Id = Id;
    Feat.Lat = Lat;
    Feat.Lng = Lng;
    if (Attrs.IsValid())
    {
        MergeAttrsInto(Feat, Attrs);
    }
    return Feat;
}

void UGeomqttClient::MergeAttrsInto(FGeomqttFeature& Feat, const TSharedPtr<FJsonObject>& Attrs)
{
    for (const auto& Pair : Attrs->Values)
    {
        FString Encoded;
        const TSharedRef<TJsonWriter<TCHAR, TCondensedJsonPrintPolicy<TCHAR>>> Writer =
            TJsonWriterFactory<TCHAR, TCondensedJsonPrintPolicy<TCHAR>>::Create(&Encoded);
        FJsonSerializer::Serialize(Pair.Value.ToSharedRef(), TEXT(""), Writer);
        // Unwrap a JSON-encoded bare string ("foo" → foo) so consumers don't
        // have to strip quotes for the common case.
        if (Pair.Value->Type == EJson::String)
        {
            Feat.Properties.Add(Pair.Key, Pair.Value->AsString());
        }
        else
        {
            Feat.Properties.Add(Pair.Key, Encoded);
        }
    }
}

void UGeomqttClient::SetViewport(const FString& Set, double CurrentZoom, const FGeomqttBbox& Bbox)
{
    LastSet = Set;
    const int32 Z = UGeomqttTileMath::ClosestPublishedZoom(CurrentZoom, PublishedZooms);
    LastZoom = Z;
    const TArray<FGeomqttTileCoord> Tiles = UGeomqttTileMath::TilesCoveringBbox(Z, Bbox);

    TSet<FString> NextTopics;
    NextTopics.Reserve(Tiles.Num());
    for (const FGeomqttTileCoord& T : Tiles)
    {
        NextTopics.Add(TileTopic(Set, T.Z, T.X, T.Y));
    }

    TArray<FString> ToSubscribe;
    TArray<FString> ToUnsubscribe;
    for (const FString& T : NextTopics)
    {
        if (!TileSubs.Contains(T)) ToSubscribe.Add(T);
    }
    for (const FString& T : TileSubs)
    {
        if (!NextTopics.Contains(T)) ToUnsubscribe.Add(T);
    }

    if (ToUnsubscribe.Num() > 0 && WebSocket.IsValid() && bMqttConnected)
    {
        TArray<uint8> Bytes;
        Mqtt::EncodeUnsubscribe(Bytes, NextPacketId++, ToUnsubscribe);
        SendBytes(Bytes);
        for (const FString& T : ToUnsubscribe) TileSubs.Remove(T);
        EvictFeaturesOutsideTopics(NextTopics, Set, Z);
    }
    if (ToSubscribe.Num() > 0 && WebSocket.IsValid() && bMqttConnected)
    {
        TArray<uint8> Bytes;
        Mqtt::EncodeSubscribe(Bytes, NextPacketId++, ToSubscribe);
        SendBytes(Bytes);
        for (const FString& T : ToSubscribe) TileSubs.Add(T);
    }
}

void UGeomqttClient::SubscribeObject(const FString& ObId)
{
    const FString Topic = FString::Printf(TEXT("objects/%s"), *ObId);
    if (ObjectSubs.Contains(Topic) || !WebSocket.IsValid() || !bMqttConnected) return;
    TArray<uint8> Bytes;
    Mqtt::EncodeSubscribe(Bytes, NextPacketId++, { Topic });
    SendBytes(Bytes);
    ObjectSubs.Add(Topic);
}

void UGeomqttClient::UnsubscribeObject(const FString& ObId)
{
    const FString Topic = FString::Printf(TEXT("objects/%s"), *ObId);
    if (!ObjectSubs.Contains(Topic) || !WebSocket.IsValid() || !bMqttConnected) return;
    TArray<uint8> Bytes;
    Mqtt::EncodeUnsubscribe(Bytes, NextPacketId++, { Topic });
    SendBytes(Bytes);
    ObjectSubs.Remove(Topic);
}

TArray<FGeomqttFeature> UGeomqttClient::GetSnapshot() const
{
    TArray<FGeomqttFeature> Out;
    Out.Reserve(Features.Num());
    for (const auto& Pair : Features) Out.Add(Pair.Value);
    return Out;
}

void UGeomqttClient::EvictFeaturesOutsideTopics(const TSet<FString>& NextTopics,
                                                const FString& Set, int32 Z)
{
    TArray<FString> ToDrop;
    for (const auto& Pair : Features)
    {
        const FGeomqttTileCoord T = UGeomqttTileMath::TileForCoord(Z, Pair.Value.Lat, Pair.Value.Lng);
        const FString Topic = TileTopic(Set, T.Z, T.X, T.Y);
        if (!NextTopics.Contains(Topic) && !ObjectSubs.Contains(FString::Printf(TEXT("objects/%s"), *Pair.Key)))
        {
            ToDrop.Add(Pair.Key);
        }
    }
    for (const FString& Id : ToDrop)
    {
        Features.Remove(Id);
        OnFeatureRemove.Broadcast(Id);
    }
}

void UGeomqttClient::SendBytes(const TArray<uint8>& Bytes)
{
    if (!WebSocket.IsValid()) return;
    WebSocket->Send(Bytes.GetData(), Bytes.Num(), /*bIsBinary=*/true);
}

bool UGeomqttClient::TickKeepalive(float /*DeltaTime*/)
{
    if (WebSocket.IsValid() && bMqttConnected)
    {
        TArray<uint8> Bytes;
        Mqtt::EncodePingReq(Bytes);
        SendBytes(Bytes);
    }
    return true; // keep ticking
}

FString UGeomqttClient::TileTopic(const FString& Set, int32 Z, int32 X, int32 Y)
{
    return FString::Printf(TEXT("geo/%s/%d/%d/%d"), *Set, Z, X, Y);
}
