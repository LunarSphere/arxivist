FROM rust:1.95-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/arxivist-indexer /usr/local/bin/arxivist-indexer

ENV BIND_ADDR=0.0.0.0:8081
EXPOSE 8081

CMD ["arxivist-indexer"]
