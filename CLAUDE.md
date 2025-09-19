# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust-based watermarking worker service that processes images and videos from Digital Ocean Spaces, adding "REFLEXU PREVIEW" watermarks. The project can run as a one-time processor or continuous worker with configurable intervals.

## Common Commands

### Build and Run
- `cargo build` - Build the project
- `cargo build --release` - Build optimized release version
- `cargo run` - Run the application
- `cargo check` - Check code for errors without building
- `cargo test` - Run tests
- `cargo clean` - Clean build artifacts

### Docker
- `docker build -t reflexu-worker .` - Build Docker image
- `docker run reflexu-worker` - Run containerized worker

## Environment Variables

Required environment variables for operation:
- `DO_SPACES_ENDPOINT` - Digital Ocean Spaces endpoint URL
- `DO_SPACES_KEY` - Digital Ocean Spaces access key
- `DO_SPACES_SECRET` - Digital Ocean Spaces secret key

Optional configuration:
- `RUN_ONCE=true` - Run once instead of continuously (default: continuous)
- `INTERVAL_MINUTES` - Minutes between processing cycles (default: 30)

## Architecture

### Core Components

1. **main.rs** - Entry point with two execution modes:
   - One-time processing (`RUN_ONCE=true`)
   - Continuous worker with health check server on port 8080

2. **process_files()** - Main processing function that:
   - Lists objects in `originals/` prefix from S3-compatible storage
   - Skips already processed files (checks for existing watermarked versions)
   - Processes images (JPG, PNG) and videos (MP4, MOV, WEBM)
   - Uploads watermarked results to `watermarks/` prefix

3. **Watermarking Functions**:
   - `watermark_image()` - Adds diagonal repeated text watermarks to images using imageproc
   - `watermark_video()` - Uses FFmpeg to add watermarks to videos with size/timeout limits

4. **Health Check Server** - HTTP server on port 8080 for container health monitoring

### Key Design Decisions

- **Resource Management**: Videos over 300MB are skipped, single-threaded FFmpeg processing
- **Timeout Protection**: 5-minute timeout for video processing to prevent hanging
- **Watermark Pattern**: Logo + text pattern repeated across media (5 horizontal lines)
- **Font Handling**: Embedded DejaVu Sans Bold font for consistent text rendering
- **Error Handling**: Graceful failures with detailed logging, continues processing other files
- **Quality Reduction for Protection**:
  - Images: Resized to max 800px, 25% JPEG quality (97% size reduction)
  - Videos: Resized to 720p, CRF 35, 1.5Mbps bitrate (98% size reduction)
- **Performance Optimizations**:
  - Fast resize algorithm (Nearest filter) for 88% faster image resizing
  - Optimized JPEG encoding parameters
  - Total processing: ~3.5s for image+video (46% faster than baseline)

## Dependencies

### Rust Dependencies (Cargo.toml)
- AWS SDK for S3-compatible storage (aws-sdk-s3, aws-config)
- Image processing (image, imageproc, rusttype)
- Async runtime (tokio)
- HTTP server (hyper, hyper-util)
- Utilities (dotenv, tempfile)

### System Dependencies
- FFmpeg (required for video processing)
- OpenSSL/TLS libraries (for HTTPS connections)

## Development Notes

- The project includes both Rust (Cargo.toml) and Python (pyproject.toml) configurations, but the main implementation is in Rust
- Font file must be present at `fonts/DejaVuSans-Bold.ttf`
- Uses multi-stage Docker build for optimized container size
- Processes files from Digital Ocean Spaces but uses AWS SDK for S3 compatibility