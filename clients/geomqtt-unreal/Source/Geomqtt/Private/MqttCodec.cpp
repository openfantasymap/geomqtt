#include "MqttCodec.h"

namespace
{
    void WriteRemainingLength(TArray<uint8>& Out, int32 Value)
    {
        do
        {
            uint8 Byte = static_cast<uint8>(Value % 128);
            Value /= 128;
            if (Value > 0) Byte |= 0x80;
            Out.Add(Byte);
        } while (Value > 0);
    }

    /** Returns: 0 = need more bytes, >0 = bytes consumed for the length field,
     *  -1 = malformed. Sets OutLength on success. */
    int32 ReadRemainingLength(const TArray<uint8>& Buf, int32 Offset, int32& OutLength)
    {
        int32 Multiplier = 1;
        int32 Value = 0;
        int32 Read = 0;
        while (true)
        {
            if (Offset + Read >= Buf.Num()) return 0;
            const uint8 Byte = Buf[Offset + Read];
            Value += (Byte & 0x7F) * Multiplier;
            ++Read;
            if ((Byte & 0x80) == 0) break;
            Multiplier *= 128;
            if (Multiplier > 128 * 128 * 128) return -1;
        }
        OutLength = Value;
        return Read;
    }

    void WriteUtf8String(TArray<uint8>& Out, const FString& Str)
    {
        const FTCHARToUTF8 Converter(*Str);
        const int32 Len = Converter.Length();
        check(Len <= 0xFFFF);
        Out.Add(static_cast<uint8>((Len >> 8) & 0xFF));
        Out.Add(static_cast<uint8>(Len & 0xFF));
        Out.Append(reinterpret_cast<const uint8*>(Converter.Get()), Len);
    }

    bool ReadUtf8String(const TArray<uint8>& Buf, int32& Cursor, int32 End, FString& Out)
    {
        if (Cursor + 2 > End) return false;
        const int32 Len = (static_cast<int32>(Buf[Cursor]) << 8) | static_cast<int32>(Buf[Cursor + 1]);
        Cursor += 2;
        if (Cursor + Len > End) return false;
        Out = FString(FUTF8ToTCHAR(reinterpret_cast<const ANSICHAR*>(&Buf[Cursor]), Len));
        Cursor += Len;
        return true;
    }
}

namespace Geomqtt::Mqtt
{
    void EncodeConnect(TArray<uint8>& Out, const FString& ClientId, uint16 KeepAliveSeconds)
    {
        TArray<uint8> Variable;
        // Protocol name: "MQTT" (length-prefixed)
        WriteUtf8String(Variable, TEXT("MQTT"));
        Variable.Add(0x04);  // protocol level (v3.1.1)
        Variable.Add(0x02);  // connect flags: clean session
        Variable.Add(static_cast<uint8>((KeepAliveSeconds >> 8) & 0xFF));
        Variable.Add(static_cast<uint8>(KeepAliveSeconds & 0xFF));
        // Payload: client id
        WriteUtf8String(Variable, ClientId);

        Out.Add(0x10);                        // CONNECT
        WriteRemainingLength(Out, Variable.Num());
        Out.Append(Variable);
    }

    void EncodeSubscribe(TArray<uint8>& Out, uint16 PacketId, const TArray<FString>& Topics)
    {
        TArray<uint8> Variable;
        Variable.Add(static_cast<uint8>((PacketId >> 8) & 0xFF));
        Variable.Add(static_cast<uint8>(PacketId & 0xFF));
        for (const FString& T : Topics)
        {
            WriteUtf8String(Variable, T);
            Variable.Add(0x00); // QoS 0
        }
        Out.Add(0x82);                        // SUBSCRIBE (type 8 + reserved flags 2)
        WriteRemainingLength(Out, Variable.Num());
        Out.Append(Variable);
    }

    void EncodeUnsubscribe(TArray<uint8>& Out, uint16 PacketId, const TArray<FString>& Topics)
    {
        TArray<uint8> Variable;
        Variable.Add(static_cast<uint8>((PacketId >> 8) & 0xFF));
        Variable.Add(static_cast<uint8>(PacketId & 0xFF));
        for (const FString& T : Topics)
        {
            WriteUtf8String(Variable, T);
        }
        Out.Add(0xA2);                        // UNSUBSCRIBE
        WriteRemainingLength(Out, Variable.Num());
        Out.Append(Variable);
    }

    void EncodePingReq(TArray<uint8>& Out)
    {
        Out.Add(0xC0);
        Out.Add(0x00);
    }

    void EncodeDisconnect(TArray<uint8>& Out)
    {
        Out.Add(0xE0);
        Out.Add(0x00);
    }

    bool TryParseNextPacket(TArray<uint8>& Buffer, EPacketType& OutType, FParsedPublish& OutPublish)
    {
        if (Buffer.Num() < 2) return false;
        const uint8 First = Buffer[0];
        const uint8 TypeNibble = (First >> 4) & 0x0F;
        int32 RemainingLen = 0;
        const int32 LenBytes = ReadRemainingLength(Buffer, 1, RemainingLen);
        if (LenBytes == 0) return false;
        if (LenBytes < 0)
        {
            Buffer.Reset();
            return false;
        }
        const int32 Total = 1 + LenBytes + RemainingLen;
        if (Buffer.Num() < Total) return false;

        const int32 PayloadStart = 1 + LenBytes;
        const int32 PayloadEnd = PayloadStart + RemainingLen;
        OutType = static_cast<EPacketType>(TypeNibble);

        switch (OutType)
        {
            case EPacketType::Publish:
            {
                int32 Cursor = PayloadStart;
                FString Topic;
                if (!ReadUtf8String(Buffer, Cursor, PayloadEnd, Topic))
                {
                    Buffer.RemoveAt(0, Total, EAllowShrinking::No);
                    return false;
                }
                // QoS 0 → no packet identifier in variable header.
                OutPublish.Topic = MoveTemp(Topic);
                const int32 PayloadLen = PayloadEnd - Cursor;
                OutPublish.Payload.Reset(PayloadLen);
                OutPublish.Payload.Append(&Buffer[Cursor], PayloadLen);
                break;
            }
            default:
                // CONNACK / SUBACK / UNSUBACK / PINGRESP — we only care that
                // they arrived. Skip the body.
                break;
        }

        Buffer.RemoveAt(0, Total, EAllowShrinking::No);
        return true;
    }
}
