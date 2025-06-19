FROM rust:1.82 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bullseye-slim
RUN apt update && apt install -y ca-certificates ffmpeg
COPY --from=builder /app/target/release/reflexu_worker_rust /worker
COPY fonts/DejaVuSans-Bold.ttf /fonts/DejaVuSans-Bold.ttf
WORKDIR /app
CMD ["/worker"]
