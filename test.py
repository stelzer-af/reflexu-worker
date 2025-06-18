import boto3
import os

s3 = boto3.client(
    "s3",
    endpoint_url=os.getenv("DO_SPACES_ENDPOINT"),
    aws_access_key_id=os.getenv("DO_SPACES_KEY"),
    aws_secret_access_key=os.getenv("DO_SPACES_SECRET"),
)

response = s3.list_buckets()
print("Buckets:", [b["Name"] for b in response["Buckets"]])
