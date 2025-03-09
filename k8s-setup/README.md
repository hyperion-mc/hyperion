# Hyperion Kubernetes/Terraform Setup

This repository contains configuration files for deploying the Hyperion Minecraft server stack using Kubernetes and Terraform.

## Architecture Overview

The deployment architecture follows these key principles:

1. **Network Separation**:
   - The tag and hyperion-proxy services run on the same network/nodes
   - The rust-mc-bot service runs on separate nodes

2. **Scalability**:
   - Each component can be scaled independently
   - Horizontal scaling for the proxy layer
   - Vertical and horizontal scaling for the bot layer

3. **Infrastructure as Code**:
   - All infrastructure defined in Terraform
   - Kubernetes resources defined as YAML manifests

## Directory Structure

```
k8s-setup/
├── kubernetes/           # Kubernetes manifests
│   ├── hyperion-namespace.yaml
│   ├── hyperion-configmap.yaml
│   ├── tag-deployment.yaml
│   ├── hyperion-proxy-deployment.yaml
│   ├── rust-mc-bot-deployment.yaml
│   └── README.md
├── terraform/            # Terraform configurations
│   ├── main.tf           # Generic Kubernetes setup
│   ├── aws-eks.tf        # AWS EKS specific setup
│   └── README.md
└── README.md             # This file
```

## Network Architecture

```
                           ┌───────────────────────┐
                           │   Kubernetes Cluster  │
                           │                       │
                           │  ┌─────────────────┐  │
                           │  │                 │  │
┌────────────┐             │  │   Core Nodes    │  │
│            │             │  │ ┌─────────────┐ │  │
│  External  │             │  │ │    tag      │ │  │
│  Players   │◄────────────┼──┼─┤   Service   │ │  │
│            │    Port     │  │ │  (35565)    │ │  │
└────────────┘   25565     │  │ └──────┬──────┘ │  │
                           │  │        │        │  │
                           │  │ ┌──────▼──────┐ │  │
                           │  │ │ hyperion-   │ │  │
                           │  │ │   proxy     │ │  │
                           │  │ │  Service    │ │  │
                           │  │ └──────┬──────┘ │  │
                           │  └────────┼────────┘  │
                           │           │           │
                           │  ┌────────▼────────┐  │
                           │  │                 │  │
                           │  │    Bot Nodes    │  │
                           │  │ ┌─────────────┐ │  │
                           │  │ │ rust-mc-bot │ │  │
                           │  │ │  Service    │ │  │
                           │  │ └─────────────┘ │  │
                           │  │                 │  │
                           │  └─────────────────┘  │
                           │                       │
                           └───────────────────────┘
```

## Deployment Options

You have two options for deploying this setup:

1. **Manual Kubernetes Deployment**:
   - Use the manifests in the `kubernetes/` directory
   - Apply them with kubectl
   - See `kubernetes/README.md` for step-by-step instructions

2. **Terraform Automated Deployment**:
   - Use the configurations in the `terraform/` directory
   - Choose between generic Kubernetes or AWS EKS deployment
   - See `terraform/README.md` for detailed instructions

## Scaling Considerations

- **Proxy Scaling**: Add more replicas of the hyperion-proxy service to handle more concurrent connections
- **Bot Scaling**: Increase the number of rust-mc-bot pods as needed
- **Resource Limits**: Adjust CPU and memory limits based on usage patterns

## Getting Started

1. Clone this repository
2. Choose your deployment method (Kubernetes or Terraform)
3. Follow the instructions in the respective README files
4. Connect to your Minecraft server using the external IP of the hyperion-proxy service

## Requirements

- Kubernetes cluster (v1.20+)
- Terraform (v1.0.0+ for Terraform deployment)
- kubectl configured with access to your cluster 