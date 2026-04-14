# ABOUTME: Variables for the centralized dravr-artifacts Terraform configuration
# ABOUTME: Populate env_app_sa_emails after applying dev/prod environment Terraform

project_id = "dravr-artifacts"

env_app_sa_emails = [
  # Populate after applying dev/prod environments:
  "dravr-mcp-server-app@dravr-dev.iam.gserviceaccount.com",
  "service-865150413606@serverless-robot-prod.iam.gserviceaccount.com",
  "terraform-runner@dravr-dev.iam.gserviceaccount.com",
  # "pierre-mcp-server-app@dravr-prod.iam.gserviceaccount.com",
]
