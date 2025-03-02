terraform {
  required_providers {
    kubernetes = {
      source = "hashicorp/kubernetes"
      version = "~> 2.23.0"
    }
  }
}

# Configure the Kubernetes provider
provider "kubernetes" {
  # Use a kubeconfig file or cloud provider specific configuration
  # config_path = "~/.kube/config" # Uncomment for local development
}

# Define resource for hyperion namespace
resource "kubernetes_namespace" "hyperion" {
  metadata {
    name = "hyperion"
    labels = {
      name = "hyperion"
    }
  }
}

# Create labels for node selection
locals {
  node_labels = {
    "hyperion-proxy-tag" = "role=core"
    "hyperion-bot" = "role=bot"
  }
}

# Use kubernetes_manifest for node labels (requires kubectl apply)
resource "null_resource" "label_nodes" {
  # Define a node selection logic - this would be customized based on your specific environment
  # This is a placeholder - in real implementation, you would use cloud provider specific node tagging
  
  provisioner "local-exec" {
    command = <<-EOT
      # Label your core node(s) that run tag & proxy
      kubectl label nodes YOUR_CORE_NODE_NAME role=core --overwrite
      
      # Label your bot node(s)
      kubectl label nodes YOUR_BOT_NODE_NAME role=bot --overwrite
    EOT
  }

  depends_on = [kubernetes_namespace.hyperion]
}

# Apply all Kubernetes manifests
resource "null_resource" "apply_kubernetes_manifests" {
  provisioner "local-exec" {
    command = <<-EOT
      kubectl apply -f ../kubernetes/hyperion-namespace.yaml
      kubectl apply -f ../kubernetes/hyperion-configmap.yaml
      kubectl apply -f ../kubernetes/tag-deployment.yaml
      kubectl apply -f ../kubernetes/hyperion-proxy-deployment.yaml
      kubectl apply -f ../kubernetes/rust-mc-bot-deployment.yaml
    EOT
  }

  depends_on = [null_resource.label_nodes]
}

# Output the service endpoints
output "hyperion_proxy_endpoint" {
  value = "Once deployed, the Minecraft server will be accessible at: <EXTERNAL_IP>:25565"
  description = "The endpoint where players can connect to the Minecraft server"
}

output "tag_service_endpoint" {
  value = "tag.hyperion.svc.cluster.local:35565"
  description = "Internal endpoint for the tag service"
}

output "important_note" {
  value = "Make sure to replace YOUR_CORE_NODE_NAME and YOUR_BOT_NODE_NAME with actual node names in your cluster"
} 