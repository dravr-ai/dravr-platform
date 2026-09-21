# ABOUTME: Inputs for the frontend_domain module — the hostnames to serve and the Cloud Run service behind them
# ABOUTME: Every hostname lands on one load balancer; the list is what a second name costs (nothing)

variable "project_id" {
  description = "GCP project ID that owns the load balancer, certificates and the Cloud Run service"
  type        = string
}

variable "region" {
  description = "Region of the Cloud Run service the serverless NEG points at"
  type        = string
}

variable "name_prefix" {
  description = "Prefix for every resource name (the environment's service_name, e.g. dravr-mcp-server)"
  type        = string
}

variable "cloud_run_service_name" {
  description = "Name of the Cloud Run service the load balancer fronts (the nginx frontend, ingress ALL)"
  type        = string
}

variable "domains" {
  description = "Public hostnames to terminate TLS for, all routed to the same Cloud Run service. Each gets its own Google-managed certificate validated by DNS authorization, so the certificate is ACTIVE before any A record exists."
  type        = list(string)

  validation {
    condition     = length(var.domains) > 0
    error_message = "domains must name at least one hostname; gate the module with count at the call site instead of passing an empty list."
  }

  validation {
    condition     = alltrue([for d in var.domains : can(regex("^([a-z0-9]([a-z0-9-]*[a-z0-9])?\\.)+[a-z]{2,}$", d))])
    error_message = "Each domain must be a lowercase DNS hostname such as app.dravr.ai."
  }
}

variable "labels" {
  description = "Labels applied to the resources that accept them (addresses, certificates, DNS authorizations, certificate map)"
  type        = map(string)
  default     = {}
}
