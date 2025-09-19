use aws_sdk_s3::{Client, config::Region, types::ObjectCannedAcl};
use aws_sdk_s3::config::Credentials;
use std::{env, path::PathBuf, process::Command, io::Cursor, time::Instant};
use regex::Regex;
use dotenv::dotenv;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage, imageops};
use imageproc::drawing::draw_text_mut;
use rusttype::{Font, Scale};
use tempfile::NamedTempFile;
use tokio::fs;
use aws_config::BehaviorVersion;
use tokio::time::{sleep, Duration};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, body::Incoming as IncomingBody};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    // Check if we should run in local test mode (only if explicitly set)
    if env::var("TEST_LOCAL").unwrap_or_default() == "true" {
        println!("🧪 Running in local test mode with assets folder");
        return test_local_files().await;
    }

    // Check if we should run once or continuously
    let run_once_env = env::var("RUN_ONCE").unwrap_or_default();
    let run_once = run_once_env == "true";

    println!("🔧 RUN_ONCE environment variable: '{}' (parsed as: {})", run_once_env, run_once);

    if run_once {
        println!("▶️  Running in one-time mode");
        process_files().await?;
    } else {
        // Run continuously with configurable interval
        let interval_minutes = env::var("INTERVAL_MINUTES")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .unwrap_or(30);
        
        println!("🔄 Starting continuous worker (interval: {} minutes)", interval_minutes);
        
        // Start health check server
        tokio::spawn(start_health_server());
        
        loop {
            match process_files().await {
                Ok(_) => println!("✅ Processing cycle completed"),
                Err(e) => eprintln!("❌ Processing cycle failed: {}", e),
            }
            
            println!("⏳ Waiting {} minutes until next cycle...", interval_minutes);
            sleep(Duration::from_secs(interval_minutes * 60)).await;
        }
    }
    
    Ok(())
}

async fn process_files() -> Result<(), Box<dyn std::error::Error>> {

    let bucket = "reflexu";

    // Discover all UUID directories under users/
    let uuids = discover_uuids(bucket).await?;

    if uuids.is_empty() {
        println!("ℹ️  No UUID directories found in users/");
        return Ok(());
    }

    println!("📁 Found {} UUID directories to process", uuids.len());

    for uuid in uuids {
        println!("🔄 Processing UUID: {}", uuid);
        let originals_prefix = format!("users/{}/originals/", uuid);
        let watermarks_prefix = format!("users/{}/watermarks/", uuid);

        match process_files_in_paths(bucket, &originals_prefix, &watermarks_prefix).await {
            Ok(_) => println!("✅ Completed processing UUID: {}", uuid),
            Err(e) => {
                eprintln!("❌ Failed to process UUID {}: {}", uuid, e);
                // Continue processing other UUIDs
                continue;
            }
        }
    }

    Ok(())
}

async fn discover_uuids(bucket: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let region = Region::new("nyc3");
    let endpoint_url = env::var("DO_SPACES_ENDPOINT")
        .map_err(|_| "DO_SPACES_ENDPOINT environment variable not found")?;
    let access_key = env::var("DO_SPACES_KEY")
        .map_err(|_| "DO_SPACES_KEY environment variable not found")?;
    let secret_key = env::var("DO_SPACES_SECRET")
        .map_err(|_| "DO_SPACES_SECRET environment variable not found")?;

    let credentials = Credentials::new(access_key, secret_key, None, None, "do-spaces");

    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(region)
        .endpoint_url(endpoint_url)
        .credentials_provider(credentials)
        .build();

    let client = Client::from_conf(s3_config);

    // List objects under users/ with delimiter to get UUID directories
    let objects = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix("users/")
        .delimiter("/")
        .send()
        .await?;

    let uuid_regex = Regex::new(r"^users/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/$")?;
    let mut uuids = Vec::new();

    // Check common prefixes (directories)
    for prefix in objects.common_prefixes() {
        if let Some(prefix_str) = prefix.prefix() {
            if uuid_regex.is_match(prefix_str) {
                // Extract UUID from "users/{uuid}/"
                let uuid = prefix_str.strip_prefix("users/").unwrap().trim_end_matches('/');
                uuids.push(uuid.to_string());
            }
        }
    }

    println!("🔍 Discovered {} valid UUID directories", uuids.len());
    for uuid in &uuids {
        println!("   📁 {}", uuid);
    }

    Ok(uuids)
}

async fn process_files_in_paths(bucket: &str, originals_prefix: &str, watermarks_prefix: &str) -> Result<(), Box<dyn std::error::Error>> {

    let region = Region::new("nyc3");
    let endpoint_url = env::var("DO_SPACES_ENDPOINT")
        .map_err(|_| "DO_SPACES_ENDPOINT environment variable not found")?;
    let access_key = env::var("DO_SPACES_KEY")
        .map_err(|_| "DO_SPACES_KEY environment variable not found")?;
    let secret_key = env::var("DO_SPACES_SECRET")
        .map_err(|_| "DO_SPACES_SECRET environment variable not found")?;

    let credentials = Credentials::new(access_key, secret_key, None, None, "do-spaces");

    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(region)
        .endpoint_url(endpoint_url)
        .credentials_provider(credentials)
        .build();

    let client = Client::from_conf(s3_config);

    let objects = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(originals_prefix)
        .send()
        .await?;

    for obj in objects.contents() {
            let key = obj.key().unwrap();
            if key.ends_with('/') { continue; }

            let path = PathBuf::from(key);
            let filename = path.file_name().unwrap().to_str().unwrap();
            let ext = path.extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let base = filename.trim_end_matches(&format!(".{}", ext));
            let dest_key = format!("{}{}-watermark.{}", watermarks_prefix, base, ext);

            if client.head_object().bucket(bucket).key(&dest_key).send().await.is_ok() {
                println!("⏭️  Skipping already watermarked: {}", filename);
                continue;
            }

            println!("📥 Downloading: {}", key);
            let object = client.get_object().bucket(bucket).key(key).send().await?;
            let body = object.body.collect().await?.into_bytes();

            match ext.to_lowercase().as_str() {
                "jpg" | "jpeg" | "png" => {
                    let img = image::load_from_memory(&body)?;
                    let (orig_width, orig_height) = img.dimensions();

                    // Resize image to max 800px for preview (lower quality for protection)
                    let max_dimension = 800u32;
                    let resized_img = if orig_width > max_dimension || orig_height > max_dimension {
                        let ratio = if orig_width > orig_height {
                            max_dimension as f32 / orig_width as f32
                        } else {
                            max_dimension as f32 / orig_height as f32
                        };
                        let new_width = (orig_width as f32 * ratio) as u32;
                        let new_height = (orig_height as f32 * ratio) as u32;
                        println!("📐 Resizing image from {}x{} to {}x{}", orig_width, orig_height, new_width, new_height);
                        // Use Triangle filter for much faster resizing with good quality
                        img.resize(new_width, new_height, imageops::FilterType::Triangle)
                    } else {
                        println!("📐 Image size {}x{} is already optimal", orig_width, orig_height);
                        img
                    };

                    println!("🖋️ Watermarking image...");
                    let watermarked = watermark_image(resized_img, "REFLEXU PREVIEW");

                    let mut buf = Cursor::new(Vec::new());
                    // Very low JPEG quality (25%) to discourage unauthorized use
                    watermarked.write_to(&mut buf, image::ImageOutputFormat::Jpeg(25))?;
                    let final_bytes = buf.into_inner();

                    client.put_object()
                        .bucket(bucket)
                        .key(&dest_key)
                        .body(final_bytes.into())
                        .acl(ObjectCannedAcl::PublicRead)
                        .send()
                        .await?;
                }
                "mp4" | "mov" | "webm" => {
                    // Skip very large videos to avoid resource issues
                    let file_size_mb = body.len() as f64 / 1024.0 / 1024.0;
                    if file_size_mb > 300.0 {
                        eprintln!("⚠️  Skipping large video ({}MB): {}", file_size_mb as u32, filename);
                        continue;
                    }
                    
                    println!("🎬 Watermarking video ({:.1}MB)...", file_size_mb);
                    
                    // Add timeout to prevent hanging
                    let timeout_duration = Duration::from_secs(300); // 5 minutes max
                    let content = match tokio::time::timeout(timeout_duration, watermark_video(&body, "REFLEXU PREVIEW")).await {
                        Ok(Ok(v)) => {
                            println!("✅ Video watermarking completed, size: {} bytes", v.len());
                            v
                        },
                        Ok(Err(e)) => {
                            eprintln!("❌ Failed to watermark video {}: {}", filename, e);
                            continue;
                        },
                        Err(_) => {
                            eprintln!("❌ Video watermarking timed out after 5 minutes: {}", filename);
                            continue;
                        }
                    };

                    println!("📤 Uploading watermarked video to: {}", dest_key);
                    match client.put_object()
                        .bucket(bucket)
                        .key(&dest_key)
                        .body(content.into())
                        .acl(ObjectCannedAcl::PublicRead)
                        .send()
                        .await {
                        Ok(_) => println!("✅ Video upload completed: {}", dest_key),
                        Err(e) => {
                            eprintln!("❌ Failed to upload video {}: {}", dest_key, e);
                            continue;
                        }
                    };
                }
                _ => {
                    println!("❌ Unsupported file type: {}", filename);
                    continue;
                }
            }

            println!("✅ Uploaded: {}", dest_key);
        }

    Ok(())
}

fn watermark_image(img: DynamicImage, _text: &str) -> DynamicImage {
    let (width, height) = img.dimensions();
    let font_data = include_bytes!("../fonts/DejaVuSans-Bold.ttf") as &[u8];
    let font = Font::try_from_bytes(font_data).unwrap();
    let mut rgba: RgbaImage = img.to_rgba8();

    // Load the logo image
    let logo_img = match image::open("assets/logo.png") {
        Ok(img) => img,
        Err(_) => {
            eprintln!("⚠️  Could not load logo.png, using text-only watermark");
            return watermark_image_text_only(img, "www.reflexu.com");
        }
    };

    // Calculate watermark element sizes - much more subtle
    let logo_width = (width as f32 * 0.04).max(25.0) as u32; // Much smaller logo (4% of width)
    let logo_height = (logo_width as f32 * logo_img.height() as f32 / logo_img.width() as f32) as u32;

    // Resize logo to watermark size
    let resized_logo = logo_img.resize(logo_width, logo_height, imageops::FilterType::Lanczos3);
    let logo_rgba = resized_logo.to_rgba8();

    // Text settings
    let text = "www.reflexu.com";
    let font_size = (logo_width as f32 * 0.6).max(10.0); // Smaller font relative to logo
    let scale = Scale::uniform(font_size);

    // Calculate text dimensions
    let text_width = text.len() as f32 * font_size * 0.6; // Approximate text width
    let dash_width = font_size * 0.3; // Width of dash character

    // Calculate pattern dimensions for subtle coverage
    // Use only 50% of image width for the watermark (increased for more spacing)
    let available_width = (width as f32 * 0.5) as i32;
    let gap = (available_width - (2 * logo_width as i32) - text_width as i32 - (2 * dash_width as i32)) / 6; // More gaps for dashes
    let pattern_width = logo_width as i32 + gap + dash_width as i32 + gap + text_width as i32 + gap + dash_width as i32 + gap + logo_width as i32;

    // Calculate center positions
    let center_x = width as i32 / 2;
    let center_y = height as i32 / 2;

    // Create 5 horizontal lines for better coverage
    let line_spacing = (height as f32 * 0.12) as i32; // Spacing between lines
    let total_pattern_height = line_spacing * 4; // 4 gaps between 5 lines
    let start_y = center_y - total_pattern_height / 2;

    for line in 0..5 {
        let y = start_y + line * line_spacing;

        // Center the pattern horizontally
        let pattern_start_x = center_x - pattern_width / 2;

        // Draw left logo
        let left_logo_x = pattern_start_x;
        let left_logo_y = y - (logo_height as i32 / 2); // Center logo vertically on the line

        if left_logo_x >= 0 && left_logo_x + logo_width as i32 <= width as i32 &&
           left_logo_y >= 0 && left_logo_y + logo_height as i32 <= height as i32 {
            draw_logo(&mut rgba, &logo_rgba, left_logo_x, left_logo_y, 0.7); // Higher opacity
        }

        // Draw left dash
        let left_dash_x = pattern_start_x + logo_width as i32 + gap;
        let left_dash_y = y - (font_size as i32 / 2); // Center dash vertically on the line

        if left_dash_x >= 0 && left_dash_x + dash_width as i32 <= width as i32 &&
           left_dash_y >= 0 && left_dash_y + font_size as i32 <= height as i32 {
            draw_text_mut(
                &mut rgba,
                Rgba([255, 255, 255, 150]), // Higher opacity
                left_dash_x,
                left_dash_y,
                scale,
                &font,
                "-"
            );
        }

        // Draw center text
        let text_x = pattern_start_x + logo_width as i32 + gap + dash_width as i32 + gap;
        let text_y = y - (font_size as i32 / 2); // Center text vertically on the line

        if text_x >= 0 && text_x + text_width as i32 <= width as i32 &&
           text_y >= 0 && text_y + font_size as i32 <= height as i32 {
            draw_text_mut(
                &mut rgba,
                Rgba([255, 255, 255, 150]), // Higher opacity
                text_x,
                text_y,
                scale,
                &font,
                text
            );
        }

        // Draw right dash
        let right_dash_x = pattern_start_x + logo_width as i32 + gap + dash_width as i32 + gap + text_width as i32 + gap;
        let right_dash_y = y - (font_size as i32 / 2); // Center dash vertically on the line

        if right_dash_x >= 0 && right_dash_x + dash_width as i32 <= width as i32 &&
           right_dash_y >= 0 && right_dash_y + font_size as i32 <= height as i32 {
            draw_text_mut(
                &mut rgba,
                Rgba([255, 255, 255, 150]), // Higher opacity
                right_dash_x,
                right_dash_y,
                scale,
                &font,
                "-"
            );
        }

        // Draw right logo
        let right_logo_x = pattern_start_x + logo_width as i32 + gap + dash_width as i32 + gap + text_width as i32 + gap + dash_width as i32 + gap;
        let right_logo_y = y - (logo_height as i32 / 2); // Center logo vertically on the line

        if right_logo_x >= 0 && right_logo_x + logo_width as i32 <= width as i32 &&
           right_logo_y >= 0 && right_logo_y + logo_height as i32 <= height as i32 {
            draw_logo(&mut rgba, &logo_rgba, right_logo_x, right_logo_y, 0.7); // Higher opacity
        }
    }

    DynamicImage::ImageRgba8(rgba)
}

fn draw_logo(canvas: &mut RgbaImage, logo: &RgbaImage, x: i32, y: i32, opacity: f32) {
    let (canvas_width, canvas_height) = canvas.dimensions();
    let (logo_width, logo_height) = logo.dimensions();

    for logo_y in 0..logo_height {
        for logo_x in 0..logo_width {
            let canvas_x = x + logo_x as i32;
            let canvas_y = y + logo_y as i32;

            // Check bounds
            if canvas_x >= 0 && canvas_x < canvas_width as i32 &&
               canvas_y >= 0 && canvas_y < canvas_height as i32 {

                let logo_pixel = logo.get_pixel(logo_x, logo_y);
                let canvas_pixel = canvas.get_pixel_mut(canvas_x as u32, canvas_y as u32);

                // Alpha blend with opacity
                let logo_alpha = (logo_pixel[3] as f32 / 255.0) * opacity;
                let inv_alpha = 1.0 - logo_alpha;

                canvas_pixel[0] = (canvas_pixel[0] as f32 * inv_alpha + logo_pixel[0] as f32 * logo_alpha) as u8;
                canvas_pixel[1] = (canvas_pixel[1] as f32 * inv_alpha + logo_pixel[1] as f32 * logo_alpha) as u8;
                canvas_pixel[2] = (canvas_pixel[2] as f32 * inv_alpha + logo_pixel[2] as f32 * logo_alpha) as u8;
            }
        }
    }
}

fn watermark_image_text_only(img: DynamicImage, text: &str) -> DynamicImage {
    let (width, height) = img.dimensions();
    let font_data = include_bytes!("../fonts/DejaVuSans-Bold.ttf") as &[u8];
    let font = Font::try_from_bytes(font_data).unwrap();
    let mut rgba: RgbaImage = img.to_rgba8();

    let center_x = width as i32 / 2;
    let center_y = height as i32 / 2;

    // Diagonal repeated watermarks only
    let diagonal_font_size = (width.min(height) as f32 * 0.05).max(16.0);
    let diagonal_scale = Scale::uniform(diagonal_font_size);

    let x_step = (width as f32 / 2.5) as i32;
    let y_step = (height as f32 / 3.0) as i32;

    // Apply rotation effect by drawing at diagonal positions
    for y in (-(height as i32)..(height as i32) * 2).step_by(y_step as usize) {
        for x in (-(width as i32)..(width as i32) * 2).step_by(x_step as usize) {
            // Calculate rotated position (simulate -30 degree rotation)
            let cos_30 = 0.866f32; // cos(-π/6)
            let sin_30 = -0.5f32;  // sin(-π/6)

            let rotated_x = ((x as f32 * cos_30 - y as f32 * sin_30) as i32) + center_x;
            let rotated_y = ((x as f32 * sin_30 + y as f32 * cos_30) as i32) + center_y;

            // Only draw if within image bounds
            if rotated_x > 0 && rotated_x < width as i32 - 100 &&
               rotated_y > 0 && rotated_y < height as i32 - 30 {
                draw_text_mut(
                    &mut rgba,
                    Rgba([255, 255, 255, 80]), // Semi-transparent white
                    rotated_x,
                    rotated_y,
                    diagonal_scale,
                    &font,
                    text
                );
            }
        }
    }

    DynamicImage::ImageRgba8(rgba)
}

async fn watermark_video(input_bytes: &[u8], watermark_text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let input_file = NamedTempFile::with_suffix(".mp4")?.into_temp_path();
    let output_file = NamedTempFile::with_suffix(".mp4")?.into_temp_path();

    println!("📁 Input file: {}", input_file.display());
    println!("📁 Output file: {}", output_file.display());
    println!("📊 Input size: {} bytes", input_bytes.len());

    fs::write(&input_file, input_bytes).await?;
    println!("✅ Wrote input file successfully");

    println!("🎬 Starting ffmpeg process...");

    // Create highly visible watermarks that actually show up in video
    // 5 lines with high opacity and large font size
    let mut watermark_filters = Vec::new();

    // Create 5 lines with pattern similar to images but text-based for FFmpeg
    for line in 0..5 {
        let y_position = format!("h/2 + (h*0.12)*({} - 2)", line); // Match image spacing

        // Left "REFLEXU" text - much more visible with stroke for thickness
        watermark_filters.push(format!(
            "drawtext=text='REFLEXU':fontcolor=white@0.6:fontsize=h/40:borderw=2:bordercolor=white@0.3:x=w*0.2:y={}",
            y_position
        ));

        // Left dash
        watermark_filters.push(format!(
            "drawtext=text='-':fontcolor=white@0.6:fontsize=h/40:borderw=2:bordercolor=white@0.3:x=w*0.32:y={}",
            y_position
        ));

        // Center "www.reflexu.com" text - much bigger and more opaque with stroke for thickness
        watermark_filters.push(format!(
            "drawtext=text='www.reflexu.com':fontcolor=white@0.6:fontsize=h/40:borderw=2:bordercolor=white@0.3:x=w/2-tw/2:y={}",
            y_position
        ));

        // Right dash
        watermark_filters.push(format!(
            "drawtext=text='-':fontcolor=white@0.6:fontsize=h/40:borderw=2:bordercolor=white@0.3:x=w*0.68:y={}",
            y_position
        ));

        // Right "REFLEXU" text
        watermark_filters.push(format!(
            "drawtext=text='REFLEXU':fontcolor=white@0.6:fontsize=h/40:borderw=2:bordercolor=white@0.3:x=w*0.8-tw:y={}",
            y_position
        ));
    }

    let watermark_filter = watermark_filters.join(",");
    
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-y",
        "-i", input_file.to_str().unwrap(),
        "-vf", &format!("scale=1280:-1,{}", watermark_filter), // Scale down to 1280px width (720p)
        "-c:v", "libx264",
        "-crf", "35", // Moderate quality reduction
        "-preset", "ultrafast",
        "-threads", "1", // Single thread to reduce resource usage
        "-b:v", "1500k", // Limit bitrate to 1.5Mbps
        "-movflags", "+faststart", // Optimize for streaming
        "-an", // No audio
        output_file.to_str().unwrap(),
    ]);
    
    let ffmpeg_output = cmd.output()?;
    
    println!("🎬 FFmpeg process completed");

    if !ffmpeg_output.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
        let stdout = String::from_utf8_lossy(&ffmpeg_output.stdout);
        eprintln!("❌ FFmpeg failed with exit code: {}", ffmpeg_output.status.code().unwrap_or(-1));
        eprintln!("❌ FFmpeg stderr: {}", stderr);
        eprintln!("❌ FFmpeg stdout: {}", stdout);
        return Err(format!("FFmpeg command failed with exit code: {}", ffmpeg_output.status.code().unwrap_or(-1)).into());
    }

    // Check if output file exists and has content
    if !output_file.exists() {
        return Err("Output file was not created by ffmpeg".into());
    }

    let result_bytes = fs::read(&output_file).await?;
    println!("📊 Output size: {} bytes", result_bytes.len());
    
    if result_bytes.is_empty() {
        return Err("Output file is empty".into());
    }

    Ok(result_bytes)
}

async fn start_health_server() {
    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("🔧 Health check server listening on port 8080");

    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(health_handler))
                .await
            {
                println!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn health_handler(_req: Request<IncomingBody>) -> Result<Response<String>, hyper::Error> {
    Ok(Response::new("OK".to_string()))
}

async fn test_local_files() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Starting local test mode...");
    let total_start = Instant::now();

    // Create output directory for watermarked files
    let output_dir = PathBuf::from("assets/watermarked");
    if !output_dir.exists() {
        fs::create_dir(&output_dir).await?;
        println!("📁 Created output directory: {}", output_dir.display());
    }

    // Read all files from assets directory
    let assets_dir = PathBuf::from("assets");
    let mut entries = fs::read_dir(&assets_dir).await?;

    let mut processed_count = 0;
    let mut total_processing_time = 0.0;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        // Skip directories and the watermarked output directory
        if path.is_dir() || path.file_name().unwrap() == "watermarked" {
            continue;
        }

        let filename = path.file_name().unwrap().to_str().unwrap();
        let ext = path.extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();

        // Skip the logo file since it's used for watermarking
        if filename == "logo.png" {
            println!("⏭️  Skipping logo file (used for watermarking): {}", filename);
            continue;
        }

        println!("\n📂 Processing: {}", filename);
        let file_start = Instant::now();

        match ext.as_str() {
            "jpg" | "jpeg" | "png" => {
                println!("🖼️  Processing image: {}", filename);
                let read_start = Instant::now();
                let body = fs::read(&path).await?;
                println!("   Read time: {:.2}ms", read_start.elapsed().as_secs_f64() * 1000.0);

                let decode_start = Instant::now();
                let img = image::load_from_memory(&body)?;
                let (orig_width, orig_height) = img.dimensions();
                println!("   Decode time: {:.2}ms ({}x{})", decode_start.elapsed().as_secs_f64() * 1000.0, orig_width, orig_height);

                // Resize image to max 800px for preview (lower quality for protection)
                let resize_start = Instant::now();
                let max_dimension = 800u32;
                let resized_img = if orig_width > max_dimension || orig_height > max_dimension {
                    let ratio = if orig_width > orig_height {
                        max_dimension as f32 / orig_width as f32
                    } else {
                        max_dimension as f32 / orig_height as f32
                    };
                    let new_width = (orig_width as f32 * ratio) as u32;
                    let new_height = (orig_height as f32 * ratio) as u32;
                    println!("📐 Resizing from {}x{} to {}x{}", orig_width, orig_height, new_width, new_height);
                    // Use Triangle filter for much faster resizing with good quality
                    let resized = img.resize(new_width, new_height, imageops::FilterType::Triangle);
                    println!("   Resize time: {:.2}ms", resize_start.elapsed().as_secs_f64() * 1000.0);
                    resized
                } else {
                    println!("📐 Image size {}x{} is already optimal", orig_width, orig_height);
                    img
                };

                println!("🖋️  Applying watermark...");
                let watermark_start = Instant::now();
                let watermarked = watermark_image(resized_img, "REFLEXU PREVIEW");
                println!("   Watermark time: {:.2}ms", watermark_start.elapsed().as_secs_f64() * 1000.0);

                let output_path = output_dir.join(format!("{}-watermarked.jpg",
                    path.file_stem().unwrap().to_str().unwrap()));

                let encode_start = Instant::now();
                let mut buf = Cursor::new(Vec::new());
                watermarked.write_to(&mut buf, image::ImageOutputFormat::Jpeg(85))?;
                println!("   Encode time: {:.2}ms", encode_start.elapsed().as_secs_f64() * 1000.0);

                let write_start = Instant::now();
                fs::write(&output_path, buf.into_inner()).await?;
                println!("   Write time: {:.2}ms", write_start.elapsed().as_secs_f64() * 1000.0);

                let file_time = file_start.elapsed().as_secs_f64();
                println!("✅ Saved watermarked image: {} (Total: {:.2}s)", output_path.display(), file_time);
                processed_count += 1;
                total_processing_time += file_time;
            }
            "mp4" | "mov" | "webm" => {
                println!("🎥 Processing video: {}", filename);
                let read_start = Instant::now();
                let body = fs::read(&path).await?;
                let file_size_mb = body.len() as f64 / 1024.0 / 1024.0;
                println!("   Read time: {:.2}s", read_start.elapsed().as_secs_f64());

                if file_size_mb > 300.0 {
                    println!("⚠️  Skipping large video ({}MB): {}", file_size_mb as u32, filename);
                    continue;
                }

                println!("🎬 Watermarking video ({:.1}MB)...", file_size_mb);

                let watermark_start = Instant::now();
                let timeout_duration = Duration::from_secs(300);
                let watermarked = match tokio::time::timeout(timeout_duration, watermark_video(&body, "REFLEXU PREVIEW")).await {
                    Ok(Ok(v)) => {
                        println!("   Watermark time: {:.2}s", watermark_start.elapsed().as_secs_f64());
                        println!("✅ Video watermarking completed");
                        v
                    },
                    Ok(Err(e)) => {
                        eprintln!("❌ Failed to watermark video {}: {}", filename, e);
                        continue;
                    },
                    Err(_) => {
                        eprintln!("❌ Video watermarking timed out: {}", filename);
                        continue;
                    }
                };

                let write_start = Instant::now();
                let output_path = output_dir.join(format!("{}-watermarked.{}",
                    path.file_stem().unwrap().to_str().unwrap(), ext));
                fs::write(&output_path, watermarked).await?;
                println!("   Write time: {:.2}s", write_start.elapsed().as_secs_f64());

                let file_time = file_start.elapsed().as_secs_f64();
                println!("✅ Saved watermarked video: {} (Total: {:.2}s)", output_path.display(), file_time);
                processed_count += 1;
                total_processing_time += file_time;
            }
            _ => {
                println!("⏭️  Skipping unsupported file: {}", filename);
            }
        }
    }

    let total_time = total_start.elapsed().as_secs_f64();
    println!("\n{}", "=".repeat(60));
    println!("📊 PERFORMANCE SUMMARY");
    println!("{}", "=".repeat(60));
    println!("📁 Files processed: {}", processed_count);
    println!("⏱️  Total execution time: {:.2}s", total_time);
    println!("⚡ Average time per file: {:.2}s", if processed_count > 0 { total_processing_time / processed_count as f64 } else { 0.0 });
    println!("🔄 Processing time only: {:.2}s", total_processing_time);
    println!("🔧 Overhead time: {:.2}s", total_time - total_processing_time);
    println!("{}", "=".repeat(60));
    println!("🎉 Local test completed! Check assets/watermarked/ for results");
    Ok(())
}
