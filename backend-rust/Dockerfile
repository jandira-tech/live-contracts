# ---- build stage ----
FROM rust:1-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential clang llvm-dev libclang-dev pkg-config && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN cargo build --release

# ---- runtime stage ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sec-ex10-rust /usr/local/bin/sec-ex10-rust

ENV PORT=7860
ENV RUST_LOG=info

EXPOSE 7860

USER 1000
CMD ["sec-ex10-rust"]
