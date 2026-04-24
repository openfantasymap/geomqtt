#!/bin/sh
# Polls the open-notify ISS position API every $INTERVAL seconds and writes
# the position into geomqtt via its RESP-compatible endpoint. Demo only —
# the open-notify endpoint is unauthenticated and rate-limited; don't lean
# on it for anything real.
set -eu

: "${GEOMQTT_HOST:=geomqtt}"
: "${GEOMQTT_PORT:=6380}"
: "${GEOMQTT_SET:=iss}"
: "${GEOMQTT_OBID:=iss}"
: "${GEOMQTT_OBJ_PREFIX:=obj:}"
: "${INTERVAL:=5}"
: "${ISS_URL:=http://api.open-notify.org/iss-now.json}"

echo "iss-demo: poll=${ISS_URL} every ${INTERVAL}s -> ${GEOMQTT_HOST}:${GEOMQTT_PORT} set=${GEOMQTT_SET} obid=${GEOMQTT_OBID}"

# One-shot HSET so /objects/<obid> shows useful properties even before the
# first move event lands.
redis-cli -h "$GEOMQTT_HOST" -p "$GEOMQTT_PORT" \
    HSET "${GEOMQTT_OBJ_PREFIX}${GEOMQTT_OBID}" \
    type satellite \
    icon iss \
    color white \
    name "International Space Station" >/dev/null || true

while :; do
    if RESP=$(curl -fsS --max-time 4 "$ISS_URL"); then
        LAT=$(printf '%s' "$RESP" | jq -r '.iss_position.latitude // empty')
        LON=$(printf '%s' "$RESP" | jq -r '.iss_position.longitude // empty')
        TS=$(printf '%s'  "$RESP" | jq -r '.timestamp // empty')
        if [ -n "$LAT" ] && [ -n "$LON" ]; then
            redis-cli -h "$GEOMQTT_HOST" -p "$GEOMQTT_PORT" \
                GEOADD "$GEOMQTT_SET" "$LON" "$LAT" "$GEOMQTT_OBID" >/dev/null
            if [ -n "$TS" ]; then
                redis-cli -h "$GEOMQTT_HOST" -p "$GEOMQTT_PORT" \
                    HSET "${GEOMQTT_OBJ_PREFIX}${GEOMQTT_OBID}" timestamp "$TS" >/dev/null
            fi
            echo "iss-demo: lat=${LAT} lon=${LON} ts=${TS}"
        else
            echo "iss-demo: malformed response: $RESP" >&2
        fi
    else
        echo "iss-demo: fetch failed (will retry)" >&2
    fi
    sleep "$INTERVAL"
done
