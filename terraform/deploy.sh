#!/bin/bash

# Colors for better readability
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${YELLOW}Minecraft Bot Testing Infrastructure Deployment${NC}"
echo

# Check if Terraform is installed
if ! command -v terraform &> /dev/null; then
    echo -e "${RED}Terraform is not installed. Please install it first.${NC}"
    exit 1
fi

# Check if HCLOUD_TOKEN is set
if [ -z "$HCLOUD_TOKEN" ]; then
    echo -e "${YELLOW}Hetzner Cloud API token not found in environment variables.${NC}"
    echo -e "Please enter your Hetzner Cloud API token:"
    read -s token
    export HCLOUD_TOKEN=$token
fi

# Create terraform.tfvars if it doesn't exist
if [ ! -f terraform.tfvars ]; then
    echo -e "${YELLOW}Creating terraform.tfvars file...${NC}"
    cat > terraform.tfvars << EOF
hcloud_token = "$HCLOUD_TOKEN"
ssh_key_name = "minecraft-bot-test"
# Uncomment and modify the variables below if you want to override defaults
#location = "nbg1"
#image = "ubuntu-22.04"
#game_server_type = "cpx31"
#proxy_server_type = "cpx21"
#bot_server_type = "cpx31"
#proxy_server_count = 2
#bot_server_count = 10
#bots_per_server = 10000
EOF
    echo -e "${GREEN}terraform.tfvars created. Edit this file if you need to customize any variables.${NC}"
fi

# Initialize Terraform
echo -e "${YELLOW}Initializing Terraform...${NC}"
terraform init

# Validate configuration
echo -e "${YELLOW}Validating Terraform configuration...${NC}"
terraform validate

if [ $? -ne 0 ]; then
    echo -e "${RED}Terraform validation failed. Please fix the errors and try again.${NC}"
    exit 1
fi

# Plan the deployment
echo -e "${YELLOW}Planning the deployment...${NC}"
terraform plan -out=tfplan

# Ask for confirmation
echo
echo -e "${YELLOW}Do you want to apply this plan? (y/n)${NC}"
read -r answer

if [[ "$answer" =~ ^[Yy]$ ]]; then
    echo -e "${YELLOW}Applying the Terraform plan...${NC}"
    terraform apply tfplan
    
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}Deployment successful!${NC}"
        echo -e "${YELLOW}Server information:${NC}"
        terraform output
    else
        echo -e "${RED}Deployment failed.${NC}"
        exit 1
    fi
else
    echo -e "${YELLOW}Deployment canceled.${NC}"
fi 