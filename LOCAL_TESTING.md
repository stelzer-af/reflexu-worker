# Local Testing Guide

This document explains how to run and benchmark the watermarking functionality locally using test assets.

## Prerequisites

- Rust and Cargo installed
- FFmpeg installed (for video processing)
- Test assets in the `assets/` folder

## Running Local Tests

### Basic Local Test

To test the watermarking functionality with local files:

```bash
TEST_LOCAL=true cargo run
```

This will:
- Process all images and videos in the `assets/` folder
- Apply "REFLEXU PREVIEW" watermarks
- Save results to `assets/watermarked/`
- Show detailed timing information for performance analysis

### Clean Test Run

To ensure consistent results, clean the output directory before each test:

```bash
rm -rf assets/watermarked && TEST_LOCAL=true cargo run
```

## Performance Benchmarking

### 📊 Current Performance Baseline:
- **Total execution:** 9.08s
- **3 files processed:** 3.03s average per file
- **Breakdown:**
  - `test.MP4` (33.3MB): 2.10s (mostly FFmpeg processing)
  - `test.JPG` (2.5MB): 6.60s (decode: 1.6s, watermark: 1s, encode: 4s)
  - `logo.png` (115KB): 0.38s (very fast)

### Key Performance Insights

**Current bottlenecks identified:**
1. JPEG encoding takes 4s (60% of image processing time)
2. JPEG decoding takes 1.6s (24% of image processing time)
3. Watermarking itself is relatively fast (~1s for large images)

### Timing Breakdown Details

For each file processed, you'll see:

**Images:**
- Read time (file I/O)
- Decode time (image parsing)
- Watermark time (applying watermark)
- Encode time (JPEG compression)
- Write time (saving result)

**Videos:**
- Read time (file I/O)
- Watermark time (FFmpeg processing)
- Write time (saving result)

### Performance Summary Output

At the end of each run, you'll get:
```
============================================================
📊 PERFORMANCE SUMMARY
============================================================
📁 Files processed: 3
⏱️  Total execution time: 9.08s
⚡ Average time per file: 3.03s
🔄 Processing time only: 9.08s
🔧 Overhead time: 0.00s
============================================================
```

## Benchmarking Workflow

1. **Baseline measurement**: Run the test before making changes
2. **Record results**: Note the total time and breakdown
3. **Make changes**: Implement your optimizations
4. **Compare**: Run the test again and compare results

## Supported File Types

- **Images**: JPG, JPEG, PNG
- **Videos**: MP4, MOV, WEBM (up to 300MB)

## Tips for Performance Testing

- Run tests multiple times for consistent results
- Close other applications to minimize system load
- Use the same test files for comparable results
- Monitor CPU and memory usage during processing
- Test with different file sizes to identify scaling issues