# geomqtt bench harness
#
# A self-contained container that runs the bench/ Python CLI against a
# geomqtt instance reachable at the GEOMQTT_BENCH_*_HOST environment
# variables (default: in-cluster service name `geomqtt`). Results are
# written to /results which compose mounts to ./results on the host.
#
# Build context is the bench/ directory itself:
#   docker build -t geomqtt-bench:local bench/
#
# (Compose handles this for you — see docker-compose.bench.yml.)

FROM python:3.12-slim AS runtime

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1 \
    GEOMQTT_BENCH_RESULTS_DIR=/results \
    GEOMQTT_BENCH_RESP_HOST=geomqtt:6380 \
    GEOMQTT_BENCH_MQTT_HOST=geomqtt:1883 \
    GEOMQTT_BENCH_MQTT_WS_HOST=geomqtt:8083 \
    GEOMQTT_BENCH_HTTP_HOST=geomqtt:8080

WORKDIR /app

COPY requirements.txt ./
RUN pip install -r requirements.txt

# Copy the bench package into /app/bench so `python -m bench` resolves.
COPY __init__.py __main__.py /app/bench/
COPY scenarios/ /app/bench/scenarios/

RUN mkdir -p /results

# Default to the smoke scenario so a `docker compose run --rm bench`
# without args is non-destructive. Override at run time:
#   docker compose run --rm bench all
ENTRYPOINT ["python", "-m", "bench"]
CMD ["smoke"]
