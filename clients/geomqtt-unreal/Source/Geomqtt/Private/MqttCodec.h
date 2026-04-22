// Minimal MQTT v3.1.1 codec — only the packet types geomqtt needs:
// outgoing CONNECT / SUBSCRIBE / UNSUBSCRIBE / PINGREQ, incoming CONNACK /
// SUBACK / UNSUBACK / PUBLISH / PINGRESP. QoS 0 only, clean session only.

#pragma once

#include "CoreMinimal.h"

namespace Geomqtt::Mqtt
{
    enum class EPacketType : uint8
    {
        Connect     = 1,
        ConnAck     = 2,
        Publish     = 3,
        Subscribe   = 8,
        SubAck      = 9,
        Unsubscribe = 10,
        UnsubAck    = 11,
        PingReq     = 12,
        PingResp    = 13,
        Disconnect  = 14,
    };

    struct FParsedPublish
    {
        FString Topic;
        TArray<uint8> Payload;
    };

    /** Append a single MQTT packet to OutBytes. */
    void EncodeConnect(TArray<uint8>& OutBytes, const FString& ClientId, uint16 KeepAliveSeconds);
    void EncodeSubscribe(TArray<uint8>& OutBytes, uint16 PacketId, const TArray<FString>& Topics);
    void EncodeUnsubscribe(TArray<uint8>& OutBytes, uint16 PacketId, const TArray<FString>& Topics);
    void EncodePingReq(TArray<uint8>& OutBytes);
    void EncodeDisconnect(TArray<uint8>& OutBytes);

    /** Try to parse the next complete packet at the front of the rolling buffer.
     *  On success: fills OutType (and OutPublish if it's a PUBLISH), drops the
     *  consumed bytes from the front of `Buffer`, returns true. On "need more
     *  bytes": returns false and leaves Buffer unchanged. On malformed input:
     *  returns false and clears Buffer. */
    bool TryParseNextPacket(TArray<uint8>& Buffer, EPacketType& OutType, FParsedPublish& OutPublish);
}
