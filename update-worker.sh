#!/bin/bash
echo "Stopping service..."
systemctl stop reflexu-worker

echo "Pulling latest code..."
cd /root/reflexu-worker
git pull origin main

echo "Building..."
cargo build --release

echo "Starting service..."
systemctl start reflexu-worker

echo "Update complete! Checking status..."
systemctl status reflexu-worker