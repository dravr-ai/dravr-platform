# ABOUTME: Global external Application Load Balancer that puts first-party hostnames (app.dravr.ai, mcp.dravr.ai) in front of the nginx frontend Cloud Run service
# ABOUTME: TLS terminates here on Google-managed certificates validated by DNS authorization; nginx and the backend's internal-only ingress are unchanged

# Why a load balancer and not a Cloud Run domain mapping: domain mappings are not
# offered in northamerica-northeast1, and a proxied Cloudflare CNAME cannot reach
# Cloud Run either (the proxy keeps the client Host for SNI, and overriding it is
# Enterprise-only). The ALB is a TLS front door bolted on ahead of nginx; nginx
# keeps every job it has today (it is the only thing that can reach the
# internal-only backend, it is the SPA's same-origin API, and it owns the path
# routing, the SPA fallback and the security headers).
#
# One load balancer, one IPv4/IPv6 pair, one backend serves every hostname in
# var.domains. The certificate map picks the certificate off SNI, so a second
# hostname adds a certificate and a map entry but no forwarding rule, and
# therefore no cost. Global forwarding rules are billed as a bundle of five;
# the four here (https/http × v4/v6) fit inside it.
#
# Certificates are validated by DNS authorization rather than by the classic
# managed certificate's load-balancer check, so the certificate reaches ACTIVE
# before the hostname's A record exists. That collapses the cutover to a single
# DNS change with no window during which the name resolves to a proxy without a
# valid certificate. The authorization records to publish are exposed as an
# output; the run.app hostname keeps serving throughout.

locals {
  # "app.dravr.ai" -> "app-dravr-ai": the hostname as a GCP resource-name segment.
  domain_slug = { for d in var.domains : d => replace(d, ".", "-") }
}

# -----------------------------------------------------------------------------
# Backend: the Cloud Run frontend behind a serverless NEG
# -----------------------------------------------------------------------------

resource "google_compute_region_network_endpoint_group" "frontend" {
  name                  = "${var.name_prefix}-frontend-neg"
  project               = var.project_id
  region                = var.region
  network_endpoint_type = "SERVERLESS"

  cloud_run {
    service = var.cloud_run_service_name
  }
}

resource "google_compute_backend_service" "frontend" {
  name                  = "${var.name_prefix}-frontend"
  project               = var.project_id
  load_balancing_scheme = "EXTERNAL_MANAGED"
  protocol              = "HTTPS"
  # A serverless NEG backend takes no health check: Cloud Run reports its own
  # readiness and the backend service must not reference one.
  timeout_sec = var.backend_timeout_sec

  backend {
    group = google_compute_region_network_endpoint_group.frontend.id
  }
}

# -----------------------------------------------------------------------------
# Certificates: one Google-managed certificate per hostname, DNS-authorized
# -----------------------------------------------------------------------------

resource "google_certificate_manager_dns_authorization" "domain" {
  for_each = toset(var.domains)

  name     = "${var.name_prefix}-${local.domain_slug[each.value]}-dns-auth"
  project  = var.project_id
  location = "global"
  domain   = each.value
  labels   = var.labels
}

resource "google_certificate_manager_certificate" "domain" {
  for_each = toset(var.domains)

  name     = "${var.name_prefix}-${local.domain_slug[each.value]}"
  project  = var.project_id
  location = "global"
  labels   = var.labels

  managed {
    domains            = [each.value]
    dns_authorizations = [google_certificate_manager_dns_authorization.domain[each.value].id]
  }
}

resource "google_certificate_manager_certificate_map" "frontend" {
  name    = "${var.name_prefix}-frontend"
  project = var.project_id
  labels  = var.labels
}

resource "google_certificate_manager_certificate_map_entry" "domain" {
  for_each = toset(var.domains)

  name         = local.domain_slug[each.value]
  project      = var.project_id
  map          = google_certificate_manager_certificate_map.frontend.name
  hostname     = each.value
  certificates = [google_certificate_manager_certificate.domain[each.value].id]
  labels       = var.labels
}

# -----------------------------------------------------------------------------
# Frontend: URL maps, proxies, addresses, forwarding rules
# -----------------------------------------------------------------------------

resource "google_compute_url_map" "frontend" {
  name            = "${var.name_prefix}-frontend"
  project         = var.project_id
  default_service = google_compute_backend_service.frontend.id
}

# Port 80 answers with a permanent redirect to https:// on the same host and
# path; nothing is served in clear.
resource "google_compute_url_map" "http_redirect" {
  name    = "${var.name_prefix}-frontend-http-redirect"
  project = var.project_id

  default_url_redirect {
    https_redirect         = true
    redirect_response_code = "MOVED_PERMANENTLY_DEFAULT"
    strip_query            = false
  }
}

resource "google_compute_target_https_proxy" "frontend" {
  name            = "${var.name_prefix}-frontend-https"
  project         = var.project_id
  url_map         = google_compute_url_map.frontend.id
  certificate_map = "//certificatemanager.googleapis.com/${google_certificate_manager_certificate_map.frontend.id}"
}

resource "google_compute_target_http_proxy" "http_redirect" {
  name    = "${var.name_prefix}-frontend-http"
  project = var.project_id
  url_map = google_compute_url_map.http_redirect.id
}

resource "google_compute_global_address" "ipv4" {
  name       = "${var.name_prefix}-frontend-ipv4"
  project    = var.project_id
  ip_version = "IPV4"
  labels     = var.labels
}

resource "google_compute_global_address" "ipv6" {
  name       = "${var.name_prefix}-frontend-ipv6"
  project    = var.project_id
  ip_version = "IPV6"
  labels     = var.labels
}

resource "google_compute_global_forwarding_rule" "https_ipv4" {
  name                  = "${var.name_prefix}-frontend-https-ipv4"
  project               = var.project_id
  load_balancing_scheme = "EXTERNAL_MANAGED"
  ip_protocol           = "TCP"
  port_range            = "443"
  ip_address            = google_compute_global_address.ipv4.id
  target                = google_compute_target_https_proxy.frontend.id
}

resource "google_compute_global_forwarding_rule" "https_ipv6" {
  name                  = "${var.name_prefix}-frontend-https-ipv6"
  project               = var.project_id
  load_balancing_scheme = "EXTERNAL_MANAGED"
  ip_protocol           = "TCP"
  port_range            = "443"
  ip_address            = google_compute_global_address.ipv6.id
  target                = google_compute_target_https_proxy.frontend.id
}

resource "google_compute_global_forwarding_rule" "http_ipv4" {
  name                  = "${var.name_prefix}-frontend-http-ipv4"
  project               = var.project_id
  load_balancing_scheme = "EXTERNAL_MANAGED"
  ip_protocol           = "TCP"
  port_range            = "80"
  ip_address            = google_compute_global_address.ipv4.id
  target                = google_compute_target_http_proxy.http_redirect.id
}

resource "google_compute_global_forwarding_rule" "http_ipv6" {
  name                  = "${var.name_prefix}-frontend-http-ipv6"
  project               = var.project_id
  load_balancing_scheme = "EXTERNAL_MANAGED"
  ip_protocol           = "TCP"
  port_range            = "80"
  ip_address            = google_compute_global_address.ipv6.id
  target                = google_compute_target_http_proxy.http_redirect.id
}
