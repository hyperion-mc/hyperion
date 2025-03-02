# Hyperion Kubernetes Setup

This directory contains Kubernetes manifests for deploying the Hyperion Minecraft server components.

## Architecture

This setup includes:

- **Tag & Hyperion Proxy**: These run on the same network in the Kubernetes cluster, labeled as "core" nodes
- **Rust-MC-Bot**: Runs on separate nodes, labeled as "bot" nodes

## Deployment

### Manual Deployment

1. First, create the namespace:
   ```
   kubectl apply -f hyperion-namespace.yaml
   ```

2. Apply the ConfigMap:
   ```
   kubectl apply -f hyperion-configmap.yaml
   ```

3. Deploy the tag service:
   ```
   kubectl apply -f tag-deployment.yaml
   ```

4. Deploy the hyperion-proxy service:
   ```
   kubectl apply -f hyperion-proxy-deployment.yaml
   ```

5. Deploy the rust-mc-bot service:
   ```
   kubectl apply -f rust-mc-bot-deployment.yaml
   ```

### Terraform Deployment

Refer to the `../terraform` directory for automated deployment using Terraform.

## Network Configuration

- **Tag and Proxy**: These services communicate over the Kubernetes cluster network
- **Proxy Service**: Exposed as a LoadBalancer to allow external connections
- **Bot Service**: Configured to run on separate nodes with the `role=bot` label

## Scaling

- To scale the proxy layer horizontally:
  ```
  kubectl scale deployment hyperion-proxy -n hyperion --replicas=3
  ```

- To scale the bot layer:
  ```
  kubectl scale deployment rust-mc-bot -n hyperion --replicas=10
  ```

## Exposing the Service

The hyperion-proxy service is exposed as a LoadBalancer, which will provide an external IP address. Players can connect to this IP address on port 25565.

To get the external IP:
```
kubectl get svc hyperion-proxy -n hyperion
```

## Customization

- Modify resource limits in the deployment files as needed
- Update environment variables in the ConfigMap 