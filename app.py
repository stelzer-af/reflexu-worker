import os
import boto3
import filetype
import tempfile
from io import BytesIO
from PIL import Image, ImageDraw, ImageFont
import ffmpeg
from dotenv import load_dotenv
load_dotenv()

BUCKET = "reflexu"
ORIGINALS_PREFIX = "originals/"
WATERMARKS_PREFIX = "watermarks/"
WATERMARK_TEXT = "REFLEXU PREVIEW"

s3 = boto3.client(
    "s3",
    endpoint_url=os.getenv("DO_SPACES_ENDPOINT"),
    aws_access_key_id=os.getenv("DO_SPACES_KEY"),
    aws_secret_access_key=os.getenv("DO_SPACES_SECRET"),
)

def list_originals():
    paginator = s3.get_paginator("list_objects_v2")
    for page in paginator.paginate(Bucket=BUCKET, Prefix=ORIGINALS_PREFIX):
        for obj in page.get("Contents", []):
            if not obj["Key"].endswith("/"):
                yield obj["Key"]

def watermark_image(data: bytes) -> bytes:
    with Image.open(BytesIO(data)) as im:
        im = im.convert("RGB")
        draw = ImageDraw.Draw(im)
        font = ImageFont.load_default()
        width, height = im.size
        draw.text((width // 2, height // 2), WATERMARK_TEXT, fill="white", font=font, anchor="mm")
        output = BytesIO()
        im.save(output, format="JPEG")
        return output.getvalue()

def watermark_video(data: bytes) -> bytes:
    with tempfile.NamedTemporaryFile(suffix=".mp4") as input_file, \
         tempfile.NamedTemporaryFile(suffix=".mp4") as output_file:

        input_file.write(data)
        input_file.flush()

        ffmpeg.input(input_file.name).drawtext(
            text=WATERMARK_TEXT,
            fontcolor="white",
            fontsize=24,
            x="(w-text_w)/2",
            y="(h-text_h)/2",
            box=1,
            boxcolor="black@0.5",
            boxborderw=5
        ).output(
            output_file.name,
            vcodec="libx264",
            crf=28,
            preset="veryfast"
        ).overwrite_output().run()

        return output_file.read()

def process_file(key: str):
    base = os.path.basename(key)
    name, ext = os.path.splitext(base)
    watermark_key = f"{WATERMARKS_PREFIX}{name}-watermark{ext}"

    # Skip if already processed
    try:
        s3.head_object(Bucket=BUCKET, Key=watermark_key)
        print(f"✅ Already exists: {watermark_key}")
        return
    except s3.exceptions.ClientError:
        pass  # Not found, continue

    # Download original file
    obj = s3.get_object(Bucket=BUCKET, Key=key)
    data = obj["Body"].read()

    kind = filetype.guess(data)
    if not kind:
        print(f"⚠️ Unknown file type: {key}")
        return

    print(f"▶️ Processing {key} as {kind.mime}...")

    if kind.mime.startswith("image/"):
        result = watermark_image(data)
        content_type = "image/jpeg"
    elif kind.mime.startswith("video/"):
        result = watermark_video(data)
        content_type = "video/mp4"
    else:
        print(f"❌ Unsupported type: {kind.mime}")
        return

    # Upload watermarked file
    s3.put_object(
        Bucket=BUCKET,
        Key=watermark_key,
        Body=result,
        ACL="public-read",
        ContentType=content_type,
    )

    public_url = f"https://{BUCKET}.{os.getenv('DO_SPACES_ENDPOINT').replace('https://', '')}/{watermark_key}"
    print(f"✅ Uploaded: {watermark_key}")
    print(f"🌐 Public URL: {public_url}")

def main():
    for key in list_originals():
        process_file(key)

if __name__ == "__main__":
    main()
