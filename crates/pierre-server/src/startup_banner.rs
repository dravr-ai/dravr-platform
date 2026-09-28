// ABOUTME: The endpoint list the server prints at startup — every mounted surface, grouped, with its local URL
// ABOUTME: Console output only; startup_banner_test pins every line against the router the server serves
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What an operator reads in the first screen of the log.
//!
//! The binary prints this once, after the router is built, so someone
//! starting the server locally can see which surfaces exist and where to
//! reach them without opening the route table. Nothing here decides
//! anything: adding a line does not mount a route, and a route mounted
//! without a line here still serves. The list is a selection, not an
//! inventory, but every line names a route the router serves, with the
//! method it answers and the path spelled exactly as the router declares
//! it; `startup_banner_test` fails on a line that drifts from the router.

use pierre_config::environment::ServerConfig;
use tracing::info;

/// One heading of the banner and the endpoints listed under it.
pub struct EndpointCategory {
    /// Heading printed above the group
    pub name: &'static str,
    /// `(description, methods, path)`: `methods` joins several methods with
    /// `/`, and `path` is the router's own pattern, placeholders included
    pub endpoints: &'static [(&'static str, &'static str, &'static str)],
}

/// Every endpoint the banner prints, in print order.
pub const ENDPOINT_CATEGORIES: &[EndpointCategory] = &[
    EndpointCategory {
        name: "MCP Protocol:",
        endpoints: &[
            ("HTTP Transport:", "POST", "/mcp"),
            ("Tool Discovery:", "GET", "/mcp/tools"),
        ],
    },
    EndpointCategory {
        name: "Authentication & OAuth:",
        endpoints: &[
            ("User Registration:", "POST", "/api/auth/register"),
            ("User Login:", "POST", "/oauth/token"),
            ("OAuth Authorize:", "GET", "/api/oauth/authorize/{provider}"),
            ("OAuth Callback:", "GET", "/api/oauth/callback/{provider}"),
            ("Provider Status:", "GET", "/api/providers"),
            (
                "OAuth Disconnect:",
                "DELETE",
                "/api/oauth/providers/{provider}/disconnect",
            ),
        ],
    },
    EndpointCategory {
        name: "OAuth 2.0 Server:",
        endpoints: &[
            ("Authorization:", "GET", "/oauth2/authorize"),
            ("Token Exchange:", "POST", "/oauth2/token"),
            ("Client Registration:", "POST", "/oauth2/register"),
        ],
    },
    EndpointCategory {
        name: "Admin Management:",
        endpoints: &[
            ("Admin Setup:", "POST", "/admin/setup"),
            ("List Users:", "GET", "/admin/users"),
            ("Generate Token:", "POST", "/admin/tokens"),
            ("List Tokens:", "GET", "/admin/tokens"),
        ],
    },
    EndpointCategory {
        name: "API Key Management:",
        endpoints: &[
            ("Create API Key:", "POST", "/api/keys"),
            ("List API Keys:", "GET", "/api/keys"),
            ("Delete API Key:", "DELETE", "/api/keys/{key_id}"),
            ("API Key Usage:", "GET", "/api/keys/{key_id}/usage"),
        ],
    },
    EndpointCategory {
        name: "Tenant Management:",
        endpoints: &[
            ("Create Tenant:", "POST", "/tenants"),
            ("List Tenants:", "GET", "/tenants"),
            ("My Tenants:", "GET", "/tenants/my"),
            ("Switch Tenant:", "POST", "/tenants/switch"),
        ],
    },
    EndpointCategory {
        name: "Dashboard & Monitoring:",
        endpoints: &[
            ("Health Check:", "GET", "/health"),
            ("Plugin Status:", "GET", "/health/plugins"),
        ],
    },
    EndpointCategory {
        name: "A2A Protocol:",
        endpoints: &[
            ("A2A Status:", "GET", "/a2a/status"),
            ("Agent Card:", "GET", "/.well-known/agent-card.json"),
            ("Clients (list/create):", "GET/POST", "/a2a/clients"),
            (
                "Client (get/delete):",
                "GET/DELETE",
                "/a2a/clients/{client_id}",
            ),
            ("Client Usage:", "GET", "/a2a/clients/{client_id}/usage"),
            (
                "Client Rate Limit:",
                "GET",
                "/a2a/clients/{client_id}/rate-limit",
            ),
        ],
    },
    EndpointCategory {
        name: "Configuration:",
        endpoints: &[
            ("Get Config:", "GET", "/config"),
            ("Update Config:", "PUT", "/config"),
            ("User Config:", "GET", "/config/user"),
            ("Update User Config:", "PUT", "/config/user"),
        ],
    },
    EndpointCategory {
        name: "Fitness Configuration:",
        endpoints: &[
            ("Get Fitness Config:", "GET", "/fitness/config"),
            ("Update Fitness Config:", "PUT", "/fitness/config"),
            ("Delete Fitness Config:", "DELETE", "/fitness/config"),
        ],
    },
    EndpointCategory {
        name: "Real-time Notifications:",
        endpoints: &[("SSE Stream:", "GET", "/notifications/sse/{user_id}")],
    },
];

/// Display all available API endpoints with their ports
pub fn display_available_endpoints(config: &ServerConfig) {
    // Default to 127.0.0.1 for local development - production uses reverse proxy
    let host = "127.0.0.1";
    let port = config.http_port;

    info!("=== Available API Endpoints ===");
    for category in ENDPOINT_CATEGORIES {
        display_endpoint_category(category, host, port);
    }
    info!("=== End of Endpoint List ===");
}

/// Display one category of endpoints with consistent formatting
fn display_endpoint_category(category: &EndpointCategory, host: &str, port: u16) {
    info!("{}", category.name);
    for (description, method, path) in category.endpoints {
        info!("   {description:18} {method} http://{host}:{port}{path}");
    }
}
