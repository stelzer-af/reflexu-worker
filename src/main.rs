use aws_sdk_s3::{Client, config::Region, types::ObjectCannedAcl};
use aws_sdk_s3::config::Credentials;
use std::{env, path::PathBuf, process::Command, io::Cursor};
use dotenv::dotenv;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use rusttype::{Font, Scale};
use tempfile::NamedTempFile;
use tokio::fs;
use aws_config::BehaviorVersion;
use tokio::time::{sleep, Duration};

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
                    println!("🎬 Watermarking video...");
                    let content = match watermark_video(&body, "REFLEXU PREVIEW").await {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("❌ Failed to watermark video {}: {}", filename, e);
                            continue;
                        }
                    };

                    client.put_object()
                        .bucket(bucket)
                        .key(&dest_key)
                        .body(content.into())
                        .acl(ObjectCannedAcl::PublicRead)
                        .send()
                        .await?;
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

    fs::write(&input_file, input_bytes).await?;

    let ffmpeg_status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i", input_file.to_str().unwrap(),
            "-vf",
            &format!(
                "drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=w/6:y=h/6,drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=w/2:y=h/6,drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=5*w/6:y=h/6,drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=w/6:y=h/2,drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=5*w/6:y=h/2,drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=w/6:y=5*h/6,drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=w/2:y=5*h/6,drawtext=text='{}':fontcolor=white@0.3:fontsize=h/15:x=5*w/6:y=5*h/6",
                watermark_text, watermark_text, watermark_text, watermark_text, watermark_text, watermark_text, watermark_text, watermark_text
            ),
            "-c:v", "libx264",
            "-crf", "28",
            "-preset", "veryfast",
            "-an",
            output_file.to_str().unwrap(),
        ])
        .status()?;

    if !ffmpeg_status.success() {
        return Err("❌ FFmpeg command failed.".into());
    }

    let result_bytes = fs::read(output_file).await?;
    Ok(result_bytes)
}
