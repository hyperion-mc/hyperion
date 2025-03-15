# Hetzner Cloud Terraform Configuration for Minecraft Bot Testing
# This sets up the infrastructure needed for testing 100,000 Minecraft bots

terraform {
  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.44.1"
    }
  }
  required_version = ">= 1.0.0"
}

# Configure the Hetzner Cloud Provider
provider "hcloud" {
  token = var.hcloud_token
}

# Create a private network for all servers to communicate
resource "hcloud_network" "minecraft_network" {
  name     = "minecraft-network"
  ip_range = "10.0.0.0/16"
}

# Create a subnet within the network
resource "hcloud_network_subnet" "minecraft_subnet" {
  network_id   = hcloud_network.minecraft_network.id
  type         = "cloud"
  network_zone = "eu-central"
  ip_range     = "10.0.1.0/24"
}

# Firewall for common rules
resource "hcloud_firewall" "common" {
  name = "common"
  
  # Allow SSH from anywhere
  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  
  # Allow ICMP (ping)
  rule {
    direction  = "in"
    protocol   = "icmp"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
}

# Firewall specifically for Minecraft and related services
resource "hcloud_firewall" "minecraft" {
  name = "minecraft"
  
  # Allow Minecraft traffic on the standard port
  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "25565"
    source_ips = ["0.0.0.0/0", "::/0"]
  }

  # Allow tag service port
  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "27750"
    source_ips = ["0.0.0.0/0", "::/0"]
  }

  # Allow internal tag service port
  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "35565"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
}

# Output the details
output "minecraft_server_ip" {
  value = hcloud_server.game_server.ipv4_address
}

output "proxy_server_ips" {
  value = {
    for server in hcloud_server.proxy_servers : server.name => server.ipv4_address
  }
}

output "bot_server_ips" {
  value = {
    for server in hcloud_server.bot_servers : server.name => server.ipv4_address
  }
}

output "private_network_id" {
  value = hcloud_network.minecraft_network.id
}

output "container_check_command" {
  description = "Commands to check Docker container status on each server"
  value = {
    tag_server = "ssh root@${hcloud_server.game_server.ipv4_address} 'docker ps -a'"
    proxy_server = "ssh root@${hcloud_server.proxy_servers[0].ipv4_address} 'docker ps -a'"
    bot_server = "ssh root@${hcloud_server.bot_servers[0].ipv4_address} 'docker ps -a'"
  }
} 