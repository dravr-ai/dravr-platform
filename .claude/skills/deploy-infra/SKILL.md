---
name: deploy-infra
description: How to deploy Dravr infrastructure and apply Cloud Run config changes. Use when editing infra/ terraform, when a merged code change is live but a Cloud Run setting (cpu, memory, min/max instances, env var, scaling) hasn't taken effect, or when asked to plan/apply infra. Explains the two-pipeline model (app binary auto-deploys on push; terraform infra config is a separate manual apply) plus the cpu/cpu_idle guardrails.
argument-hint: [plan|apply] [dev|prod]
user-invocable: true
---

# Deploy Infra Skill

Dravr has **TWO independent deploy pipelines**. Confusing them is the #1 infra footgun: merging code ships a new binary, but it runs against the *existing* service config.

## The two pipelines

| | App / binary | Terraform infra config |
|---|---|---|
| Workflow | `publish-images.yml` ("Deploy: Build & Ship (dev)") | `terraform.yml` ("Infra: Terraform") |
| Trigger | **AUTO** on `push` to main (code paths) | `plan` **AUTO** on push to `infra/**`; `apply` **MANUAL** |
| Deploys | Docker image → new Cloud Run revision (your Rust/TS code) | service config: cpu, memory, min/max instances, env vars, IAM, budgets, BigQuery |

**Consequence:** a change to `cpu` / `memory` / scaling / an env var lives in terraform and does **NOT** take effect until someone runs a manual `apply` — even though the binary auto-deployed.

## Plan (read-only, safe)

- **Auto:** any push to main touching `infra/environments/**` or `infra/modules/**` runs `terraform plan` for **dev**. The diff appears in the Actions run's **Job Summary** (no PR comment — this repo uses local squash merges, no PRs).
- **Manual:**
  ```bash
  gh workflow run terraform.yml -f environment=dev -f action=plan
  gh run list --workflow terraform.yml --limit 1    # then watch
  ```

## Apply (mutates infra — manual, gated)

`apply` **NEVER** runs on push. Always a deliberate dispatch:
```bash
gh workflow run terraform.yml -f environment=dev -f action=apply
```
- **dev:** `environment: development` — runs immediately (no required reviewer); "gated" here means *manual trigger*, not human-reviewed.
- **prod:** `apply-prod` is `if: false` (disabled scaffolding). Dev *is* the live env today.

**APPLY ORDER when a config change depends on a new binary** (e.g. a `cpu` change that needs new server code): merge code → confirm the new Cloud Run revision is serving → **then** apply terraform. Applying config against an older binary can re-trigger a regression (this is how a premature `cpu=1` could re-break boot).

## Guardrails — do not trip these

- **`cpu_idle = false` is ADR-019-gated.** `scripts/ci/check-cpu-idle-adr.sh` hard-fails any `cpu_idle = false` in `infra/**/*.tf` lacking an `ADR-<n>` ref within 8 comment lines above it. It is always-on CPU billing, owned by the Copilot/ACP runner — never flip it to chase cost without updating ADR-019.
- **`backend_cpu = 2`, not 1.** The headless-Chrome sciotte scrape (Garmin + token-less Strava) is CPU-hungry; `cpu=1` risks starving it into a tool-loop timeout. cpu=1 is *deferred* pending a Chrome-scrape load-test (async-boot fixed the boot-path blocker, NOT the steady-state Chrome need).
- **`concurrency = 1` is an OOM cap** (a turn holds a `copilot --acp` Node child + a headless Chrome vs the 2Gi budget), not a session thing. It must move in lockstep with `sciotte_max_concurrent`.
- **Never commit `tfplan` files** — they leaked tfstate secrets in this PUBLIC repo before. The CI plan writes `-out=tfplan`; keep it gitignored.

## Verify what is ACTUALLY live (applied state vs tfvars desired state)

```bash
gcloud run services describe dravr-mcp-server-api --project=dravr-dev --region=northamerica-northeast1 \
  --format="value(status.latestReadyRevisionName, \
    spec.template.spec.containers[0].resources.limits.cpu, \
    spec.template.spec.containers[0].resources.limits.memory, \
    spec.template.metadata.annotations['run.googleapis.com/cpu-throttling'])"
```
The tfvars in the repo is the **desired** state; the running service is the **applied** state. They diverge until `apply` runs.

## Reading Cloud Run logs (strip gcsfuse noise)

The dev `dravr-mcp-server-api` mounts a GCS bucket via gcsfuse, which floods logs:
```bash
gcloud run services logs read dravr-mcp-server-api --project=dravr-dev --region=northamerica-northeast1 \
  | grep -ivE 'GCSFuse|mount-id|GetStorageLayout|UniverseDomain|storageLayout'
```

## State + runner facts

- Backend: GCS, prefix `dravr-dev` (intrinsic state locking). Runner SA: `terraform-runner@dravr-dev.iam.gserviceaccount.com` via WIF/OIDC (`secrets.GCP_DEV_WIF_PROVIDER` + `GCP_DEV_TF_SA`).
- Billing: account `016750-504582-CFEC0E`, budget `300 CAD/mo` (dev) + `500 CAD/mo` account-wide. Region: `northamerica-northeast1`.
- Background: ADR-019 (`dravr-vault/Architecture/ADRs/`) + the vault note "Cloud Run CPU Config — Archaeology, Optimal Config & cpu=1 Async-Boot".
