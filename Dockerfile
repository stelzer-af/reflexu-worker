FROM rust:1.83 AS builder
WORKDIR /app

# Clean cargo cache to avoid dependency issues
RUN cargo clean 2>/dev/null || true
RUN rm -rf ~/.cargo/registry/index/* 2>/dev/null || true

COPY Cargo.toml Cargo.lock ./
RUN cargo fetch

COPY . .
RUN cargo build --release

FROM debian:bullseye-slim
RUN apt update && apt install -y ca-certificates ffmpeg
COPY --from=builder /app/target/release/reflexu_worker_rust /worker
COPY fonts/DejaVuSans-Bold.ttf /fonts/DejaVuSans-Bold.ttf
WORKDIR /app
CMD ["/worker"]
