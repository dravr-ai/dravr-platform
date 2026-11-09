# ABOUTME: Offline plan-mode tests for the frontend load balancer's Cloud Armor policy
# ABOUTME: Uses mock_provider, so no GCP credentials; Go's regexp and Cloud Armor's matcher are both RE2
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

mock_provider "google" {}

variables {
  project_id               = "test-project"
  region                   = "northamerica-northeast1"
  name_prefix              = "test"
  cloud_run_service_name   = "test-frontend"
  domains                  = ["app.example.com"]
  armor_enabled            = true
  marketplace_egress_cidrs = ["160.79.104.0/21", "153.61.208.0/20"]
}

# Every rule that can refuse a request starts in preview, the default a fresh
# apply gets. A refused webhook sender or connector tells nobody.
run "every_refusing_rule_starts_in_preview" {
  command = plan

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      r.preview == true if r.action != "allow"
    ])
    error_message = "A rule that can refuse a request must default to preview"
  }

  assert {
    condition     = length([for r in google_compute_security_policy.frontend[0].rule : r if r.action != "allow"]) == 5
    error_message = "Expected the two WAF rules, two throttles and the ban"
  }
}

run "each_preview_variable_promotes_only_its_rules" {
  command = plan

  variables {
    armor_rate_rules_preview = false
  }

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      r.preview == (r.action == "deny(403)") if r.action != "allow"
    ])
    error_message = "armor_rate_rules_preview must move the rate rules and leave the WAF in preview"
  }
}

# With the header absent an HTTP_HEADER key falls back to ALL: one counter shared
# by every connector's opening handshake.
run "the_token_throttle_never_sees_a_request_without_the_header" {
  command = plan

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      startswith(r.match[0].expr[0].expression, "has(request.headers['authorization']) &&")
      if r.action == "throttle" && r.rate_limit_options[0].enforce_on_key == "HTTP_HEADER"
    ])
    error_message = "The HTTP_HEADER throttle must match only requests that carry the header"
  }

  assert {
    condition = length([
      for r in google_compute_security_policy.frontend[0].rule : r
      if r.action == "throttle" && r.rate_limit_options[0].enforce_on_key == "IP" && startswith(r.match[0].expr[0].expression, "!has(request.headers['authorization']) &&")
    ]) == 1
    error_message = "Requests without the header must have their own per-address throttle"
  }
}

# Evaluation ends at an enforced match. Below the allow, the token throttle would
# never see the marketplace traffic it exists for; above it, the per-address rules
# would count one shared egress as one client.
run "the_marketplace_allow_sits_between_the_token_and_address_limits" {
  command = plan

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      r.rate_limit_options[0].enforce_on_key == "HTTP_HEADER" ? r.priority < 3000 : r.priority > 3000
      if contains(["throttle", "rate_based_ban"], r.action)
    ])
    error_message = "The token throttle must precede the marketplace allow, and the per-address rules follow it"
  }

  assert {
    condition = length([
      for r in google_compute_security_policy.frontend[0].rule : r
      if r.priority == 3000 && r.action == "allow" && toset(r.match[0].config[0].src_ip_ranges) == toset(var.marketplace_egress_cidrs)
    ]) == 1
    error_message = "Priority 3000 must allow exactly the marketplace ranges"
  }

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      r.priority < 3000 if startswith(r.action, "deny")
    ])
    error_message = "The WAF must run before anything can end evaluation"
  }
}

run "no_path_pattern_holds_a_group_cloud_armor_refuses" {
  command = plan

  # Cloud Armor's matcher refuses any parenthesised group in request.path.matches()
  # ("capture groups are not allowed"), which only an apply against GCP reports.
  assert {
    condition = alltrue([
      for re in [local.armor_protocol_paths, local.armor_waf_paths, local.armor_ban_exempt_paths] :
      !strcontains(re, "(")
    ])
    error_message = "A path pattern holds a group; Cloud Armor refuses it at apply"
  }
}

run "the_ban_leaves_senders_and_assets_alone_and_counts_the_app" {
  command = plan

  assert {
    condition = alltrue([
      for p in [
        "/webhooks/garmin/token/sleeps", "/webhooks/stripe", "/webhooks/contremaitre",
        "/api/messaging/webhook/slack", "/api/messaging/webhook/whatsapp", "/api/ops/slack/actions",
        "/assets/index-abc.js", "/mcp", "/a2a/message:send",
      ] : length(regexall(local.armor_ban_exempt_paths, p)) > 0
    ])
    error_message = "A webhook sender, an asset or a protocol door must not count toward the ban"
  }

  assert {
    condition = alltrue([
      for p in ["/", "/api/chat/conversations", "/oauth2/token", "/admin/users", "/.env", "/mcpx", "/webhooksx"] :
      length(regexall(local.armor_ban_exempt_paths, p)) == 0
    ])
    error_message = "The app, the OAuth doors and a scanner's paths must count toward the ban"
  }

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      strcontains(r.match[0].expr[0].expression, "urlDecode().contains('..')") if r.action == "rate_based_ban"
    ])
    error_message = "A dot segment must not turn an exempt-looking path into a way past the ban"
  }
}

# Measured peaks from one address, 30 days to 2026-11-06: 517/min (a real browser,
# assets included), 912 in 10 minutes. The ban exempts /assets, so main.tf quotes
# the lower figures outside it; the floor here holds to the higher ones. A shared
# NAT multiplies those.
run "the_limits_stay_clear_of_measured_traffic" {
  command = plan

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      r.rate_limit_options[0].rate_limit_threshold[0].count >= 1034 && r.rate_limit_options[0].ban_threshold[0].count >= 1824
      if r.action == "rate_based_ban"
    ])
    error_message = "The ban must stay at least twice the measured per-address peak"
  }

  assert {
    condition = alltrue([
      for r in google_compute_security_policy.frontend[0].rule :
      r.rate_limit_options[0].rate_limit_threshold[0].count >= 22 if r.action == "throttle"
    ])
    error_message = "A throttle must stay at least twice the measured 11/min on the protocol doors"
  }
}

run "disabled_means_no_policy_and_nothing_attached" {
  command = plan

  variables {
    armor_enabled = false
  }

  assert {
    condition     = length(google_compute_security_policy.frontend) == 0 && google_compute_backend_service.frontend.security_policy == null
    error_message = "armor_enabled = false must leave no policy and no attachment"
  }
}

run "the_policy_is_attached_when_enabled" {
  command = plan

  override_resource {
    target          = google_compute_security_policy.frontend
    override_during = plan
    values = {
      id = "projects/test-project/global/securityPolicies/test-frontend"
    }
  }

  assert {
    condition     = google_compute_backend_service.frontend.security_policy == "projects/test-project/global/securityPolicies/test-frontend"
    error_message = "The policy must be attached to the frontend backend service"
  }
}

run "a_malformed_range_is_refused" {
  command = plan

  variables {
    marketplace_egress_cidrs = ["160.79.104.0"]
  }

  expect_failures = [var.marketplace_egress_cidrs]
}

run "the_whole_internet_is_not_a_marketplace" {
  command = plan

  variables {
    marketplace_egress_cidrs = ["0.0.0.0/0"]
  }

  expect_failures = [var.marketplace_egress_cidrs]
}

run "more_ranges_than_one_rule_holds_are_refused" {
  command = plan

  variables {
    marketplace_egress_cidrs = ["10.0.0.0/8", "10.1.0.0/16", "10.2.0.0/16", "10.3.0.0/16", "10.4.0.0/16", "10.5.0.0/16", "10.6.0.0/16", "10.7.0.0/16", "10.8.0.0/16", "10.9.0.0/16", "10.10.0.0/16"]
  }

  expect_failures = [var.marketplace_egress_cidrs]
}
