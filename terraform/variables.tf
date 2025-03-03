variable "hcloud_token" {
  description = "Hetzner Cloud API Token"
  type        = string
  sensitive   = true
}

variable "ssh_key_name" {
  description = "Name of the SSH key to use for server access"
  type        = string
  default     = "default"
}

variable "location" {
  description = "Hetzner location to deploy servers"
  type        = string
  default     = "fsn1" # Falkenstein, Germany
}

variable "image" {
  description = "Server image to use"
  type        = string
  default     = "ubuntu-22.04"
}

variable "game_server_type" {
  description = "Server type for the Minecraft game server"
  type        = string
  default     = "cpx31" # 4 vCPUs, 8 GB RAM
}

variable "proxy_server_type" {
  description = "Server type for proxy servers"
  type        = string
  default     = "cpx21" # 3 vCPUs, 4 GB RAM
}

variable "bot_server_type" {
  description = "Server type for bot servers"
  type        = string
  default     = "cpx31" # 4 vCPUs, 8 GB RAM
}

variable "proxy_server_count" {
  description = "Number of proxy servers to deploy"
  type        = number
  default     = 2
}

variable "bot_server_count" {
  description = "Number of bot servers to deploy"
  type        = number
  default     = 10 # Adjust based on your testing needs
}

variable "bots_per_server" {
  description = "Number of bots to run per bot server"
  type        = number
  default     = 10000 # Adjust based on your server capacity
}

variable "docker_setup_script" {
  description = "Common script for Docker installation on all servers"
  type        = string
  default     = <<-EOT
    # Update and install Docker
    apt-get update
    apt-get install -y apt-transport-https ca-certificates curl software-properties-common
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg
    echo "deb [arch=amd64 signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null
    apt-get update
    apt-get install -y docker-ce docker-ce-cli containerd.io
    systemctl enable docker
    systemctl start docker
  EOT
} 