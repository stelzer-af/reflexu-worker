FROM rustlang/rust:nightly AS builder
WORKDIR /app

# Copy all files first
COPY . .

# Clean cargo cache to avoid dependency issues
RUN cargo clean 2>/dev/null || true
RUN rm -rf ~/.cargo/registry/index/* 2>/dev/null || true

# Build the project
RUN cargo build --release

FROM debian:bullseye-slim
RUN apt update && apt install -y ca-certificates ffmpeg
COPY --from=builder /app/target/release/reflexu_worker_rust /worker
COPY fonts/DejaVuSans-Bold.ttf /fonts/DejaVuSans-Bold.ttf
WORKDIR /app
CMD ["/worker"]
