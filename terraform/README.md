# Hyperion Terraform Setup

This directory contains Terraform configuration for deploying Hyperion Minecraft server components on Kubernetes.

## Overview

The configuration provides:

1. A Kubernetes deployment setup using either:
   - Generic Kubernetes provider (main.tf)
   - AWS EKS-specific deployment (aws-eks.tf)

2. Node separation for:
   - Core services (tag and hyperion-proxy) running on the same node group
   - Bot services (rust-mc-bot) running on a separate node group

## Prerequisites

- Terraform installed (v1.0.0+)
- AWS CLI configured (if using AWS EKS)
- kubectl installed and configured

## Usage

### Generic Kubernetes Deployment

For deploying to an existing Kubernetes cluster:

1. Initialize Terraform:
   ```
   terraform init
   ```

2. Update node names in `main.tf`:
   Replace `YOUR_CORE_NODE_NAME` and `YOUR_BOT_NODE_NAME` with actual node names.

3. Apply the configuration:
   ```
   terraform apply
   ```

### AWS EKS Deployment

For deploying to AWS EKS:

1. Initialize Terraform:
   ```
   terraform init
   ```

2. Update AWS region in `aws-eks.tf` if needed.

3. Apply the configuration:
   ```
   terraform apply
   ```

4. Configure kubectl to use the new cluster:
   ```
   aws eks update-kubeconfig --name hyperion-cluster --region us-west-2
   ```

## Architecture

- **VPC**: Dedicated VPC with public and private subnets
- **EKS Cluster**: Kubernetes cluster with separate node groups
- **Node Groups**:
  - Core nodes: For tag and hyperion-proxy services
  - Bot nodes: For rust-mc-bot services

## Customization

- Update instance types in the `eks_managed_node_groups` section
- Modify autoscaling settings (`min_size`, `max_size`, `desired_size`)
- Change AWS region or availability zones

## Cleanup

To destroy the entire infrastructure:

```
terraform destroy
``` 