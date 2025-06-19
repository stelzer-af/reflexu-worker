use aws_sdk_s3::{Client, config::Region, types::ObjectCannedAcl};
use aws_sdk_s3::config::Credentials;
use std::{env, path::PathBuf, process::Command, io::Cursor, time::Duration as StdDuration};
use dotenv::dotenv;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
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
    let originals_prefix = "originals/";
    let watermarks_prefix = "watermarks/";

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
                    println!("🖋️ Watermarking image...");
                    let watermarked = watermark_image(img, "REFLEXU PREVIEW");

                    let mut buf = Cursor::new(Vec::new());
                    watermarked.write_to(&mut buf, image::ImageOutputFormat::Jpeg(85))?;
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
                    if file_size_mb > 100.0 {
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

fn watermark_image(img: DynamicImage, text: &str) -> DynamicImage {
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
    
    // Simplified watermark - just center text to reduce complexity
    let watermark_filter = format!(
        "drawtext=text='{}':fontcolor=white@0.4:fontsize=h/20:x=(w-text_w)/2:y=(h-text_h)/2",
        watermark_text
    );
    
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-y",
        "-i", input_file.to_str().unwrap(),
        "-vf", &watermark_filter,
        "-c:v", "libx264",
        "-crf", "30", // Higher CRF for smaller file
        "-preset", "ultrafast",
        "-threads", "1", // Single thread to reduce resource usage
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
