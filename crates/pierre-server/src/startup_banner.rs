// ABOUTME: The endpoint list the server prints at startup — every mounted surface, grouped, with its local URL
// ABOUTME: Console output only; the router is the source of truth for what is actually mounted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What an operator reads in the first screen of the log.
//!
//! The binary prints this once, after the router is built, so someone
//! starting the server locally can see which surfaces exist and where to
//! reach them without opening the route table. Nothing here decides
//! anything: adding a line does not mount a route, and a route mounted
//! without a line here still serves.

use pierre_config::environment::ServerConfig;
use tracing::info;

/// Display all available API endpoints with their ports
pub fn display_available_endpoints(config: &ServerConfig) {
    // Default to 127.0.0.1 for local development - production uses reverse proxy
    let host = "127.0.0.1";

    info!("=== Available API Endpoints ===");
    display_mcp_endpoints(host, config.http_port);
    display_auth_endpoints(host, config.http_port);
    display_oauth2_endpoints(host, config.http_port);
    display_oauth_callback_urls(host, config);
    display_admin_endpoints(host, config.http_port);
    display_api_key_endpoints(host, config.http_port);
    display_tenant_endpoints(host, config.http_port);
    display_dashboard_endpoints(host, config.http_port);
    display_a2a_endpoints(host, config.http_port);
    display_config_endpoints(host, config.http_port);
    display_fitness_endpoints(host, config.http_port);
    display_notification_endpoints(host, config.http_port);
    info!("=== End of Endpoint List ===");
}

/// Endpoint category definition for structured display
struct EndpointCategory {
    name: &'static str,
    endpoints: &'static [(&'static str, &'static str, &'static str)], // (description, method, path)
}

/// Display a category of endpoints with consistent formatting
fn display_endpoint_category(category: &EndpointCategory, host: &str, port: u16) {
    info!("{}", category.name);
    for (description, method, path) in category.endpoints {
        info!("   {description:18} {method} http://{host}:{port}{path}");
    }
}

fn display_mcp_endpoints(host: &str, port: u16) {
    let endpoints = [
        "MCP Protocol:",
        &format!("   HTTP Transport:    http://{host}:{port}/mcp"),
    ];
    for line in &endpoints {
        info!("{}", line);
    }
}

fn display_auth_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Authentication & OAuth:",
        endpoints: &[
            ("User Registration:", "POST", "/auth/register"),
            ("User Login:", "POST", "/auth/login"),
            ("OAuth Authorize:", "GET", "/api/oauth/authorize/{provider}"),
            ("OAuth Callback:", "GET", "/api/oauth/callback/{provider}"),
            ("OAuth Status:", "GET", "/api/oauth/status"),
            (
                "OAuth Disconnect:",
                "POST",
                "/api/oauth/disconnect/{provider}",
            ),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_oauth2_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "OAuth 2.0 Server:",
        endpoints: &[
            ("Authorization:", "GET", "/oauth2/authorize"),
            ("Token Exchange:", "POST", "/oauth2/token"),
            ("Client Registration:", "POST", "/oauth2/register"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_oauth_callback_urls(_host: &str, config: &ServerConfig) {
    let endpoints = [
        "OAuth Callback URLs (MCP Bridge):",
        &format!(
            "   Bridge Callback:   http://localhost:{}/oauth/callback",
            config.oauth_callback_port
        ),
        &format!(
            "   Focus Recovery:    http://localhost:{}/oauth/focus-recovery",
            config.oauth_callback_port
        ),
        &format!(
            "   Provider Callback: http://localhost:{}/oauth/provider-callback/{{provider}}",
            config.oauth_callback_port
        ),
    ];
    for line in &endpoints {
        info!("{}", line);
    }
}

fn display_admin_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Admin Management:",
        endpoints: &[
            ("Admin Setup:", "POST", "/admin/setup"),
            ("Create User:", "POST", "/admin/users"),
            ("List Users:", "GET", "/admin/users"),
            ("Generate Token:", "POST", "/admin/tokens"),
            ("List Tokens:", "GET", "/admin/tokens"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_api_key_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "API Key Management:",
        endpoints: &[
            ("Create API Key:", "POST", "/api/keys"),
            ("List API Keys:", "GET", "/api/keys"),
            ("Delete API Key:", "DELETE", "/api/keys/{key_id}"),
            ("API Key Usage:", "GET", "/api/keys/usage"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_tenant_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Tenant Management:",
        endpoints: &[
            ("Create Tenant:", "POST", "/tenants"),
            ("List Tenants:", "GET", "/tenants"),
            ("Get Tenant:", "GET", "/tenants/{tenant_id}"),
            ("Update Tenant:", "PUT", "/tenants/{tenant_id}"),
            ("Delete Tenant:", "DELETE", "/tenants/{tenant_id}"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_dashboard_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Dashboard & Monitoring:",
        endpoints: &[
            ("Health Check:", "GET", "/health"),
            ("Plugin Status:", "GET", "/health/plugins"),
            ("System Status:", "GET", "/dashboard/status"),
            ("User Dashboard:", "GET", "/dashboard/user"),
            ("Admin Dashboard:", "GET", "/dashboard/admin"),
            ("Detailed Stats:", "GET", "/dashboard/detailed"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_a2a_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "A2A Protocol:",
        endpoints: &[
            ("A2A Status:", "GET", "/a2a/status"),
            ("Agent Card:", "GET", "/.well-known/agent-card.json"),
            ("Clients (list/create):", "GET/POST", "/a2a/clients"),
            ("Client (get/delete):", "GET/DELETE", "/a2a/clients/{id}"),
            ("Client Usage:", "GET", "/a2a/clients/{id}/usage"),
            ("Client Rate Limit:", "GET", "/a2a/clients/{id}/rate-limit"),
            ("Dashboard Overview:", "GET", "/a2a/dashboard/overview"),
            ("Dashboard Analytics:", "GET", "/a2a/dashboard/analytics"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_config_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Configuration:",
        endpoints: &[
            ("Get Config:", "GET", "/config"),
            ("Update Config:", "PUT", "/config"),
            ("User Config:", "GET", "/config/user"),
            ("Update User Config:", "PUT", "/config/user"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_fitness_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Fitness Configuration:",
        endpoints: &[
            ("Get Fitness Config:", "GET", "/fitness/config"),
            ("Update Fitness Config:", "PUT", "/fitness/config"),
            ("Delete Fitness Config:", "DELETE", "/fitness/config"),
        ],
    };
    display_endpoint_category(&category, host, port);
}

fn display_notification_endpoints(host: &str, port: u16) {
    let category = EndpointCategory {
        name: "Real-time Notifications:",
        endpoints: &[("SSE Stream:", "GET", "/notifications/sse?user_id={user_id}")],
    };
    display_endpoint_category(&category, host, port);
}
