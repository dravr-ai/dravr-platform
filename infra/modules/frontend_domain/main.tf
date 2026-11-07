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

  # RE2, as Cloud Armor's request.path.matches() and terraform's regexall() both
  # read them, so tests/armor.tftest.hcl replays real paths through these.
  armor_protocol_paths = "^/(mcp(/|$)|a2a/)"
  armor_waf_paths      = "^/(mcp(/|$)|a2a/|oauth2/)"
  # A sender retries from one address, and a CDN-less asset burst is one page
  # load: neither is a flood. /webhooks covers Garmin, Wahoo, COROS, Strava,
  # WHOOP, Stripe and contremaitre; each verifies its own signature or token.
  armor_ban_exempt_paths = "^/(mcp(/|$)|a2a/|webhooks/|api/messaging/webhook/|api/ops/slack/|assets/)"
}

# -----------------------------------------------------------------------------
# Cloud Armor: the edge limits of the public protocol doors
# -----------------------------------------------------------------------------
#
# Every rule that can refuse a request starts in preview (armor_waf_preview,
# armor_rate_rules_preview). A preview match is logged and evaluation goes on; an
# enforced match ends it, and a rate rule's conforming request is an enforced
# match. So the order is the design:
#
#   1000, 1100  WAF on the protocol doors, before anything can end evaluation.
#   2000        Throttle per bearer token, ABOVE the marketplace allow, because
#               every Claude athlete shares a few egress addresses and the token
#               is the only thing that tells them apart.
#   3000        Allow the marketplace ranges. Below this nothing sees them, so
#               the per-IP rules need not name the ranges.
#   3500        Throttle anonymous /mcp and /a2a per address.
#   4000        Per-address flood ban on the other paths.
#
# The header key has two traps. With no Authorization header the key type falls
# back to ALL ("The key type defaults to ALL if no such header is present"), so
# one shared counter would refuse every connector's opening 401 handshake: rule
# 2000 therefore matches only requests that carry the header, and rule 3500 keys
# the rest on the address. And the key is the first 128 bytes of the value, which
# for a Dravr JWT ends 26 characters into `sub` (Claims serializes it first; kid
# key_YYYYMMDD_HHMMSS): the throttle counts per athlete, across their tokens.
#
# Limits, against 30 days of the frontend's request log to 2026-11-06 (73,173
# requests on the load balancer's hostnames; nothing logs the header, so address
# stands in for token):
#   2000  300/min   mcp+a2a peak 11/min from one address, 2/min from one Claude one
#   3500  120/min   anonymous (401) peak 4/min from one address
#   4000  1200/min  peak 487/min (a /.env scanner), 372/min (a real browser), both
#         6000/10m  outside /assets; 836 in 10 min. A flood ceiling that leaves
#                   room for a shared NAT, not a scanner filter: over the first it
#                   answers 429 and a client retries, and only the second bans.
#
# Promoting a rule is its preview variable; the alert and the log query that show
# what it would refuse are in environments/dev/public_mcp_monitoring.tf.
resource "google_compute_security_policy" "frontend" {
  count = var.armor_enabled ? 1 : 0

  name    = "${var.name_prefix}-frontend"
  project = var.project_id
  type    = "CLOUD_ARMOR"

  # JSON-RPC and form bodies are inspected by value, not as one string. NORMAL
  # log level names the matched signature; VERBOSE would copy the matched text,
  # which can be a credential or an athlete's words, into the log.
  advanced_options_config {
    json_parsing = "STANDARD"
    log_level    = "NORMAL"
  }

  rule {
    description = "SQL injection on the protocol doors"
    priority    = 1000
    action      = "deny(403)"
    preview     = var.armor_waf_preview
    match {
      expr {
        expression = "request.path.matches('${local.armor_waf_paths}') && evaluatePreconfiguredWaf('sqli-v33-stable', {'sensitivity': 1})"
      }
    }
  }

  rule {
    description = "Cross-site scripting on the protocol doors"
    priority    = 1100
    action      = "deny(403)"
    preview     = var.armor_waf_preview
    match {
      expr {
        expression = "request.path.matches('${local.armor_waf_paths}') && evaluatePreconfiguredWaf('xss-v33-stable', {'sensitivity': 1})"
      }
    }
  }

  rule {
    description = "Throttle /mcp and /a2a per bearer token"
    priority    = 2000
    action      = "throttle"
    preview     = var.armor_rate_rules_preview
    match {
      expr {
        expression = "has(request.headers['authorization']) && request.path.matches('${local.armor_protocol_paths}')"
      }
    }
    rate_limit_options {
      conform_action      = "allow"
      exceed_action       = "deny(429)"
      enforce_on_key      = "HTTP_HEADER"
      enforce_on_key_name = "authorization"
      rate_limit_threshold {
        count        = 300
        interval_sec = 60
      }
    }
  }

  dynamic "rule" {
    for_each = length(var.marketplace_egress_cidrs) > 0 ? [var.marketplace_egress_cidrs] : []
    content {
      description = "Marketplace egress ranges: past the per-address limits"
      priority    = 3000
      action      = "allow"
      match {
        versioned_expr = "SRC_IPS_V1"
        config {
          src_ip_ranges = rule.value
        }
      }
    }
  }

  rule {
    description = "Throttle anonymous /mcp and /a2a per address"
    priority    = 3500
    action      = "throttle"
    preview     = var.armor_rate_rules_preview
    match {
      expr {
        expression = "!has(request.headers['authorization']) && request.path.matches('${local.armor_protocol_paths}')"
      }
    }
    rate_limit_options {
      conform_action = "allow"
      exceed_action  = "deny(429)"
      enforce_on_key = "IP"
      rate_limit_threshold {
        count        = 120
        interval_sec = 60
      }
    }
  }

  # The second clause: nginx resolves a dot segment, so /assets/../api/x could
  # reach the api while reading as an exempt path here.
  rule {
    description = "Per-address flood ban outside the protocol doors, webhooks and assets"
    priority    = 4000
    action      = "rate_based_ban"
    preview     = var.armor_rate_rules_preview
    match {
      expr {
        expression = "!request.path.matches('${local.armor_ban_exempt_paths}') || request.path.urlDecode().contains('..')"
      }
    }
    rate_limit_options {
      conform_action   = "allow"
      exceed_action    = "deny(429)"
      enforce_on_key   = "IP"
      ban_duration_sec = 600
      rate_limit_threshold {
        count        = 1200
        interval_sec = 60
      }
      ban_threshold {
        count        = 6000
        interval_sec = 600
      }
    }
  }

  rule {
    description = "Default: allow"
    priority    = 2147483647
    action      = "allow"
    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = ["*"]
      }
    }
  }

  # armor_enabled = false is the rollback. Without this terraform destroys the
  # policy before it updates the backend that still uses it (measured on a local
  # probe). The cost: changing the fixed name or type needs a new name, because
  # the replacement is created while the old policy still holds this one.
  lifecycle {
    create_before_destroy = true
  }
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
  # A serverless NEG backend takes no health check (Cloud Run reports its own
  # readiness) and no timeout_sec: the API refuses the field for this backend
  # type, and the timeout is a fixed, non-configurable 60 minutes — well past
  # the 600s the frontend Cloud Run service allows a chat turn.

  security_policy = var.armor_enabled ? google_compute_security_policy.frontend[0].id : null

  backend {
    group = google_compute_region_network_endpoint_group.frontend.id
  }

  # The one log of a request that never reached nginx: a certificate or routing
  # fault, or the frontend service down. Logs share the project's free monthly
  # ingestion; traffic here is small, so every request is kept.
  log_config {
    enable      = true
    sample_rate = 1.0
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
