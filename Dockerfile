FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

ARG BIN
RUN cargo build --release --bin "${BIN}"

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

ARG BIN
COPY --from=builder /app/target/release/${BIN} /usr/local/bin/arxivist-service

ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/arxivist-service"]
