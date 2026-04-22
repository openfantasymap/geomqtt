FROM rust:1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY crates ./crates
RUN cargo build --release --bin geomqtt-server

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/geomqtt-server /usr/local/bin/geomqtt-server

EXPOSE 6380 1883 8083 8080
ENV RUST_LOG=info,geomqtt_server=debug

ENTRYPOINT ["/usr/local/bin/geomqtt-server"]
