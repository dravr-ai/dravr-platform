# Pierre MCP Server - GCP Infrastructure

Terraform configuration for provisioning GCP infrastructure for Pierre MCP Server.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         GCP Project                                     │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                          VPC Network                              │  │
│  │  ┌────────────────┐     ┌────────────────┐                       │  │
│  │  │    Subnet      │     │  VPC Connector │                       │  │
│  │  │ (10.0.0.0/24)  │     │ (10.8.0.0/28)  │                       │  │
│  │  └────────────────┘     └───────┬────────┘                       │  │
│  │                                 │                                 │  │
│  │         ┌───────────────────────┼───────────────────────┐        │  │
│  │         │                       │                       │        │  │
│  │         ▼                       ▼                       ▼        │  │
│  │  ┌─────────────┐         ┌─────────────┐         ┌───────────┐  │  │
│  │  │  Cloud SQL  │         │  Cloud Run  │         │  Secret   │  │  │
│  │  │ PostgreSQL  │◄────────│   Service   │────────►│  Manager  │  │  │
│  │  │  (Private)  │         │  (Managed)  │         │           │  │  │
│  │  └─────────────┘         └─────────────┘         └───────────┘  │  │
│  │                                 ▲                                │  │
│  └─────────────────────────────────┼────────────────────────────────┘  │
│                                    │                                    │
│  ┌─────────────────────────────────┴────────────────────────────────┐  │
│  │                    Artifact Registry                              │  │
│  │                   (Docker Repository)                             │  │
│  └───────────────────────────────────────────────────────────────────┘  │
│                                    ▲                                    │
└────────────────────────────────────┼────────────────────────────────────┘
                                     │
                        ┌────────────┴────────────┐
                        │   GitHub Actions        │
                        │   (Workload Identity)   │
                        └─────────────────────────┘
```

## Prerequisites

1. **GCP Account** with billing enabled
2. **gcloud CLI** installed and authenticated
3. **Terraform** >= 1.5.0 installed
4. **GCP APIs** (enabled automatically by Terraform):
   - Compute Engine
   - Cloud SQL Admin
   - Secret Manager
   - Cloud Run
   - Cloud Build
   - Artifact Registry
   - IAM

## First-Time Setup

### 1. Create the Terraform State Bucket

Before running Terraform for the first time, create the GCS bucket for state storage:

```bash
# Set your project ID
export PROJECT_ID="pierrefitnessplatform"

# Create the state bucket
gcloud storage buckets create gs://pierre-terraform-state \
  --project=${PROJECT_ID} \
  --location=northamerica-northeast1 \
  --uniform-bucket-level-access

# Enable versioning
gcloud storage buckets update gs://pierre-terraform-state --versioning
```

### 2. Authenticate to GCP

```bash
# Login to GCP
gcloud auth login

# Set application default credentials
gcloud auth application-default login

# Set the project
gcloud config set project pierrefitnessplatform
```

### 3. Create Configuration File

```bash
# Copy the example configuration
cp terraform.tfvars.example terraform.tfvars

# Edit with your values (most defaults are fine for Pierre)
# vim terraform.tfvars
```

### 4. Initialize Terraform

```bash
cd infra
terraform init
```

### 5. Review the Plan

```bash
terraform plan
```

### 6. Apply the Configuration

```bash
terraform apply
```

## Post-Apply Steps

### Get Values for GitHub Secrets

After `terraform apply` completes, get the values needed for GitHub:

```bash
# Get all outputs
terraform output

# Get specific values for GitHub secrets
terraform output -raw workload_identity_provider
terraform output -raw deployer_service_account_email
```

Add these as GitHub repository secrets:

| Secret Name | Value From |
|-------------|------------|
| `GCP_WORKLOAD_IDENTITY_PROVIDER` | `terraform output -raw workload_identity_provider` |
| `GCP_SERVICE_ACCOUNT` | `terraform output -raw deployer_service_account_email` |

### Fill OAuth Secrets Manually

The OAuth secrets are created as placeholders. Update them with real values:

```bash
# Strava
echo -n "your-strava-client-secret" | gcloud secrets versions add pierre-mcp-server-strava-client-secret --data-file=-

# Garmin
echo -n "your-garmin-client-secret" | gcloud secrets versions add pierre-mcp-server-garmin-client-secret --data-file=-

# OpenWeather
echo -n "your-openweather-api-key" | gcloud secrets versions add pierre-mcp-server-openweather-api-key --data-file=-
```

### Publish the Public Hostnames

`public_domains` provisions the load balancer and its certificates but no DNS —
the `dravr.ai` zone lives at Cloudflare. Every record is **DNS only** (grey
cloud): a proxied record would put Cloudflare's 100s proxy timeout in front of
a chat path budgeted for 600s.

1. Apply, then read the validation CNAMEs: `terraform output public_domain_dns_authorizations`.
   Add each one at Cloudflare. The certificate validates against this record
   alone, so it reaches `ACTIVE` before the hostname resolves anywhere.
2. Wait for `gcloud certificate-manager certificates list` to show every
   certificate `ACTIVE`, then confirm the balancer serves without DNS:
   `curl -sS --resolve app.dravr.ai:443:$(terraform output -raw public_domain_ipv4) https://app.dravr.ai/health`.
3. Add the `A` (`public_domain_ipv4`) and `AAAA` (`public_domain_ipv6`) records
   for each hostname. The run.app hostname keeps serving throughout.
4. Only then move `frontend_base_url` to `https://app.dravr.ai` — that single
   variable carries `BASE_URL`, `FRONTEND_URL`, the OAuth issuer and every
   provider callback, so the provider portals and the Firebase authorized
   domains (`modules/firebase/MANUAL_STEPS.tf`) move in the same window.

## Module Reference

| Module | Description |
|--------|-------------|
| `project` | Enables required GCP APIs |
| `networking` | VPC, subnet, private service connect, VPC connector |
| `database` | Cloud SQL PostgreSQL instance, database, user |
| `secrets` | Secret Manager secrets (auto-generated and placeholders) |
| `artifact_registry` | Docker repository for container images |
| `service_accounts` | App SA (Cloud Run) and Deployer SA (GitHub Actions) |
| `workload_identity` | GitHub OIDC pool and provider |
| `storage` | Optional GCS buckets |
| `frontend_domain` | Global external ALB + Google-managed certificates for the public hostnames (`app.dravr.ai`, `mcp.dravr.ai`) in front of the frontend service |

## Configuration Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `project_id` | GCP project ID | (required) |
| `region` | GCP region | `northamerica-northeast1` |
| `environment` | Environment name | `production` |
| `service_name` | Cloud Run service name | `pierre-mcp-server` |
| `database_tier` | Cloud SQL machine tier | `db-f1-micro` |
| `github_org` | GitHub organization | `dravr-ai` |
| `github_repo` | GitHub repository | `dravr-platform` |
| `public_domains` | Public hostnames on the load balancer (empty = run.app only) | `[]` |

See `variables.tf` for full list.

## Outputs

| Output | Description |
|--------|-------------|
| `database_connection_name` | For Cloud Run `--add-cloudsql-instances` |
| `vpc_connector_id` | For Cloud Run `--vpc-connector` |
| `app_service_account_email` | For Cloud Run `--service-account` |
| `deployer_service_account_email` | For GitHub `GCP_SERVICE_ACCOUNT` secret |
| `workload_identity_provider` | For GitHub `GCP_WORKLOAD_IDENTITY_PROVIDER` secret |
| `artifact_registry_url` | For Docker push |
| `secret_ids` | Map of secret names to IDs |
| `public_domain_ipv4` / `public_domain_ipv6` | The A / AAAA record for every public hostname |
| `public_domain_dns_authorizations` | Per hostname, the CNAME that validates its certificate |

## Destroying Infrastructure

To destroy all resources:

```bash
# First, disable deletion protection on the database
terraform apply -var="database_deletion_protection=false"

# Then destroy
terraform destroy
```

**Warning**: This will delete all data including the database. Make sure to backup any important data first.

## Troubleshooting

### API Not Enabled Error

If you see "API not enabled" errors, wait 30 seconds and try again. The project module enables APIs but they may take time to propagate.

### Workload Identity Issues

If GitHub Actions fails to authenticate:

1. Verify the attribute condition matches your GitHub org:
   ```bash
   gcloud iam workload-identity-pools providers describe github-provider \
     --location=global \
     --workload-identity-pool=github-pool
   ```

2. Check the service account IAM bindings:
   ```bash
   gcloud iam service-accounts get-iam-policy \
     github-actions-deployer@pierrefitnessplatform.iam.gserviceaccount.com
   ```

### Database Connection Issues

If Cloud Run cannot connect to Cloud SQL:

1. Verify the VPC connector is running:
   ```bash
   gcloud compute networks vpc-access connectors describe pierre-vpc-connector \
     --region=northamerica-northeast1
   ```

2. Check the private service connection:
   ```bash
   gcloud compute addresses list --filter="purpose=VPC_PEERING"
   ```

## Integration with GitHub Actions

The GitHub Actions workflow (`.github/workflows/deploy-gcp.yml`) uses these Terraform outputs:

1. **Authentication**: Uses `workload_identity_provider` and `deployer_service_account_email`
2. **Build**: Pushes to `artifact_registry_url`
3. **Deploy**: Configures Cloud Run with `database_connection_name`, `vpc_connector_id`, `app_service_account_email`

The workflow handles Cloud Run service creation - Terraform only provisions infrastructure.
