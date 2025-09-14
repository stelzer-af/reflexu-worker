# Deployment Guide

This guide covers setting up and managing the Reflexu watermarking worker on a DigitalOcean droplet.

## Initial Setup

### Step 1: Connect to Droplet

```bash
ssh root@10.108.0.2
```

- First time: "Are you sure you want to continue connecting?" → Type `yes`
- Password: Enter the password you created during droplet setup

Once connected, you should see:
```
root@reflexu-worker:~#
```

### Step 2: Install Dependencies

Run these commands one by one:

```bash
# Update system packages
apt update && apt upgrade -y

# Install FFmpeg and Git
apt install -y ffmpeg git curl build-essential

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

When Rust installer asks for options, press Enter (default installation).

```bash
# Reload environment to use Rust
source ~/.cargo/env

# Verify installations
ffmpeg -version
git --version
rustc --version
```

### Step 3: Clone and Build Project

```bash
# Clone the repository
git clone <repository-url> reflexu-worker
cd reflexu-worker

# Build the release version (takes 5-10 minutes)
cargo build --release
```

### Step 4: Setup Environment Variables

Create the environment file:

```bash
nano .env
```

Add your DigitalOcean Spaces credentials:

```env
DO_SPACES_ENDPOINT=https://nyc3.digitaloceanspaces.com
DO_SPACES_KEY=your-key-here
DO_SPACES_SECRET=your-secret-here
```

Save and exit nano:
1. Press `Ctrl + X`
2. Press `Y` to confirm
3. Press `Enter` to save

### Step 5: Create Systemd Service

Create the service file:

```bash
nano /etc/systemd/system/reflexu-worker.service
```

Add the following content:

```ini
[Unit]
Description=Reflexu Watermarking Worker
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/root/reflexu-worker
ExecStart=/root/reflexu-worker/target/release/reflexu_worker_rust
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

### Step 6: Enable and Start Service

```bash
# Reload systemd to recognize the new service
systemctl daemon-reload

# Enable the service to start on boot
systemctl enable reflexu-worker

# Start the service
systemctl start reflexu-worker

# Check status
systemctl status reflexu-worker
```

## Monitoring

### View Service Status
```bash
systemctl status reflexu-worker
```

### View Live Logs
```bash
journalctl -u reflexu-worker -f
```

### Stop/Start Service
```bash
systemctl stop reflexu-worker
systemctl start reflexu-worker
systemctl restart reflexu-worker
```

## Updates

### Create Update Script

```bash
nano /root/update-worker.sh
```

Add the following content:

```bash
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
```

Make it executable:
```bash
chmod +x /root/update-worker.sh
```

### Running Updates

To update the worker:

```bash
/root/update-worker.sh
```

## Troubleshooting

### Check Service Logs
```bash
journalctl -u reflexu-worker --no-pager
```

### Check Recent Logs
```bash
journalctl -u reflexu-worker -n 50
```

### Rebuild from Scratch
```bash
systemctl stop reflexu-worker
cd /root/reflexu-worker
cargo clean
cargo build --release
systemctl start reflexu-worker
```

### Verify Environment Variables
```bash
cat /root/reflexu-worker/.env
```

## Health Check

The worker runs a health check server on port 8080. You can test it:

```bash
curl http://localhost:8080
```

Should return: `OK`