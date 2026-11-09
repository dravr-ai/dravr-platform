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

variable "armor_enabled" {
  description = "Attach a Cloud Armor security policy to the frontend backend service. It covers the hostnames in `domains` only: a request to the frontend's run.app URLs does not cross the load balancer."
  type        = bool
  default     = false
}

variable "marketplace_egress_cidrs" {
  description = "Published egress ranges of the marketplace hosts (e.g. Claude's). They pass the per-address limits, because one range carries many athletes; the per-token throttle and the WAF still apply. IPv4 or IPv6 CIDRs."
  type        = list(string)
  default     = []

  validation {
    condition     = alltrue([for c in var.marketplace_egress_cidrs : can(cidrnetmask(c)) || can(cidrhost(c, 0))])
    error_message = "marketplace_egress_cidrs must hold CIDR ranges such as 160.79.104.0/21."
  }

  validation {
    condition     = length(var.marketplace_egress_cidrs) <= 10
    error_message = "Cloud Armor allows 10 ranges per rule; a longer list needs a second allow rule."
  }

  validation {
    condition     = !contains(var.marketplace_egress_cidrs, "0.0.0.0/0") && !contains(var.marketplace_egress_cidrs, "::/0") && !contains(var.marketplace_egress_cidrs, "*")
    error_message = "marketplace_egress_cidrs must not cover the whole internet; that lifts every per-address limit."
  }
}

variable "armor_waf_preview" {
  description = "Keep the preconfigured SQL injection and XSS rules in preview: they log what they would refuse. Set false to enforce them, once the procedure in environments/dev/public_mcp_monitoring.tf shows no false positive."
  type        = bool
  default     = true
}

variable "armor_rate_rules_preview" {
  description = "Keep the per-token throttle, the anonymous throttle and the per-address ban in preview: they log what they would refuse. Set false to enforce them, once the procedure in environments/dev/public_mcp_monitoring.tf shows no false positive."
  type        = bool
  default     = true
}
