# SSH Key for server access
resource "hcloud_ssh_key" "default" {
  name       = var.ssh_key_name
  public_key = file("~/.ssh/id_rsa.pub")
}

locals {
  docker_tag_script = <<-EOT
    #!/bin/bash
    ${var.docker_setup_script}

    # Run tag service directly with Docker
    docker run -d \
      --name tag \
      --restart always \
      -p 27750:27750 \
      -p 35565:35565 \
      -e RUST_LOG=info \
      ghcr.io/hyperion-mc/hyperion/tag:latest
  EOT

  docker_proxy_script = <<-EOT
    #!/bin/bash
    ${var.docker_setup_script}

    # Get tag server IP
    TAG_IP="${hcloud_server.game_server.ipv4_address}"
    
    # Run hyperion-proxy directly with Docker
    docker run -d \
      --name hyperion-proxy \
      --restart always \
      -p 25565:25565 \
      -e RUST_LOG=info \
      ghcr.io/hyperion-mc/hyperion/hyperion-proxy:latest \
      --server "$TAG_IP:35565" "0.0.0.0:25565"
    
    # Add tag server to hosts file
    echo "$TAG_IP tag" >> /etc/hosts
  EOT

  docker_bot_script = <<-EOT
    #!/bin/bash
    ${var.docker_setup_script}

    # Set proxy IP for bot to connect to
    PROXY_IP="${hcloud_server.proxy_servers[0].ipv4_address}"
    BOTS_PER_SERVER="${var.bots_per_server}"
    
    # Run rust-mc-bot directly with Docker
    docker run -d \
      --name rust-mc-bot \
      --restart always \
      -e RUST_LOG=info \
      ghcr.io/hyperion-mc/hyperion/rust-mc-bot:latest \
      "$PROXY_IP:25565" "$BOTS_PER_SERVER" "2"
  EOT
}

# Minecraft Game Server - Now running the tag service
resource "hcloud_server" "game_server" {
  name        = "mc-game-server"
  image       = var.image
  server_type = var.game_server_type
  location    = var.location
  ssh_keys    = [hcloud_ssh_key.default.id]
  firewall_ids = [hcloud_firewall.common.id, hcloud_firewall.minecraft.id]
  
  # User data for server setup
  user_data = local.docker_tag_script
  
  # Attach to private network
  network {
    network_id = hcloud_network.minecraft_network.id
    ip         = "10.0.1.10"
  }
  
  depends_on = [
    hcloud_network_subnet.minecraft_subnet
  ]
}

# Proxy Servers - Now running hyperion-proxy service
resource "hcloud_server" "proxy_servers" {
  count       = var.proxy_server_count
  name        = "mc-proxy-${count.index + 1}"
  image       = var.image
  server_type = var.proxy_server_type
  location    = var.location
  ssh_keys    = [hcloud_ssh_key.default.id]
  firewall_ids = [hcloud_firewall.common.id, hcloud_firewall.minecraft.id]
  
  # User data for server setup
  user_data = local.docker_proxy_script
  
  # Attach to private network
  network {
    network_id = hcloud_network.minecraft_network.id
    ip         = "10.0.1.${20 + count.index}"
  }
  
  depends_on = [
    hcloud_network_subnet.minecraft_subnet,
    hcloud_server.game_server
  ]
}

# Bot Servers - Now running the rust-mc-bot service
resource "hcloud_server" "bot_servers" {
  count       = var.bot_server_count
  name        = "mc-bot-${count.index + 1}"
  image       = var.image
  server_type = var.bot_server_type
  location    = var.location
  ssh_keys    = [hcloud_ssh_key.default.id]
  firewall_ids = [hcloud_firewall.common.id]
  
  # User data for server setup
  user_data = local.docker_bot_script
  
  # Attach to private network
  network {
    network_id = hcloud_network.minecraft_network.id
    ip         = "10.0.1.${50 + count.index}"
  }
  
  depends_on = [
    hcloud_network_subnet.minecraft_subnet,
    hcloud_server.proxy_servers
  ]
} 