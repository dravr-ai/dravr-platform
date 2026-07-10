// ABOUTME: Demonstrates A2A 1.0 agent card discovery, interface + auth negotiation
// ABOUTME: Shows how agents read a peer's card before collaborating (RFC 8615 well-known path)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent Discovery Example (A2A 1.0)
//!
//! Demonstrates the A2A 1.0 discovery mechanism, showing how an agent:
//! 1. Fetches a peer's agent card from `/.well-known/agent-card.json`
//! 2. Analyzes its declared skills and protocol capabilities
//! 3. Negotiates a transport interface (preferred `supportedInterface`)
//! 4. Picks an authentication scheme from `securitySchemes`
//! 5. Makes an informed decision about whether to collaborate

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{info, warn};

/// A2A protocol version this discovering agent implements (Major.Minor).
const A2A_VERSION: &str = "1.0";

/// A2A 1.0 Agent Card (the fields this demo reads).
#[derive(Debug, Deserialize, Serialize)]
struct AgentCard {
    /// Agent name
    name: String,
    /// Human-readable description
    description: String,
    /// Agent implementation version
    version: String,
    /// Transport interfaces, ordered by preference (first = preferred)
    #[serde(rename = "supportedInterfaces")]
    supported_interfaces: Vec<AgentInterface>,
    /// Protocol feature capabilities
    capabilities: AgentCapabilities,
    /// Skills the agent offers
    skills: Vec<AgentSkill>,
    /// Named security schemes, keyed by scheme name (proto-oneof wrapped)
    #[serde(rename = "securitySchemes", default)]
    security_schemes: BTreeMap<String, serde_json::Value>,
}

/// A transport interface (A2A 1.0 `AgentInterface`).
#[derive(Debug, Deserialize, Serialize)]
struct AgentInterface {
    /// Base URL of the interface
    url: String,
    /// Protocol binding — `JSONRPC`, `GRPC`, or `HTTP+JSON`
    #[serde(rename = "protocolBinding")]
    protocol_binding: String,
    /// A2A protocol version served here (Major.Minor)
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
}

/// Protocol capabilities (A2A 1.0 `AgentCapabilities`).
#[derive(Debug, Deserialize, Serialize)]
struct AgentCapabilities {
    /// Whether streaming (SSE) is supported
    #[serde(default)]
    streaming: bool,
    /// Whether push notification webhooks are supported
    #[serde(rename = "pushNotifications", default)]
    push_notifications: bool,
    /// Whether an authenticated extended card is available
    #[serde(rename = "extendedAgentCard", default)]
    extended_agent_card: bool,
}

/// A skill offered by the agent (A2A 1.0 `AgentSkill`).
#[derive(Debug, Deserialize, Serialize)]
struct AgentSkill {
    /// Unique skill identifier
    id: String,
    /// Human-readable name
    name: String,
    /// What the skill does
    description: String,
    /// Descriptive tags
    #[serde(default)]
    tags: Vec<String>,
}

/// Agent Discovery Client
struct AgentDiscovery {
    http_client: Client,
    server_url: String,
}

impl AgentDiscovery {
    /// Create a new agent discovery client
    fn new(server_url: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("AgentDiscoveryExample/1.0")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http_client,
            server_url,
        }
    }

    /// Fetch the peer's agent card from the RFC 8615 well-known path.
    ///
    /// Discovery is public and NOT version-gated — it is how a client learns
    /// which protocol versions and interfaces the peer supports.
    async fn fetch_agent_card(&self) -> Result<AgentCard> {
        let url = format!("{}/.well-known/agent-card.json", self.server_url);
        info!("📡 Fetching agent card from: {url}");

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch agent card")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to fetch agent card: HTTP {}", response.status());
        }

        let agent_card: AgentCard = response
            .json()
            .await
            .context("Failed to parse agent card JSON")?;

        info!("✅ Successfully fetched agent card for: {}", agent_card.name);
        Ok(agent_card)
    }

    /// Analyze the peer's declared skills and protocol capabilities.
    fn analyze_capabilities(card: &AgentCard) {
        info!("\n📊 Agent Capability Analysis:");
        info!("   Agent: {} v{}", card.name, card.version);
        info!("   Description: {}", card.description);

        info!(
            "\n🔌 Transport Interfaces ({}):",
            card.supported_interfaces.len()
        );
        for (idx, interface) in card.supported_interfaces.iter().enumerate() {
            let marker = if idx == 0 { "★ preferred" } else { "alternate" };
            info!(
                "   • {} (A2A {}) at {} [{marker}]",
                interface.protocol_binding, interface.protocol_version, interface.url
            );
        }

        info!("\n⚙️  Protocol Capabilities:");
        info!("   • streaming (SSE):        {}", card.capabilities.streaming);
        info!(
            "   • push notifications:     {}",
            card.capabilities.push_notifications
        );
        info!(
            "   • extended agent card:    {}",
            card.capabilities.extended_agent_card
        );

        info!("\n🛠️  Available Skills ({}):", card.skills.len());
        for skill in &card.skills {
            info!(
                "   • {} ({}) - {}",
                skill.id,
                skill.tags.join(", "),
                skill.description
            );
        }

        info!("\n🔐 Security Schemes:");
        for name in card.security_schemes.keys() {
            info!("   • {name}");
        }
    }

    /// Check whether the agent offers a skill with the given id.
    fn has_skill(card: &AgentCard, skill_id: &str) -> bool {
        card.skills.iter().any(|skill| skill.id == skill_id)
    }

    /// Find skills matching a name/description/tag substring.
    fn find_skills<'a>(card: &'a AgentCard, pattern: &str) -> Vec<&'a AgentSkill> {
        let pattern = pattern.to_lowercase();
        card.skills
            .iter()
            .filter(|skill| {
                skill.id.to_lowercase().contains(&pattern)
                    || skill.description.to_lowercase().contains(&pattern)
                    || skill.tags.iter().any(|tag| tag.to_lowercase().contains(&pattern))
            })
            .collect()
    }

    /// Negotiate a transport interface: pick the peer's preferred binding
    /// that also serves the protocol version this agent speaks.
    fn negotiate_interface(card: &AgentCard) -> Option<&AgentInterface> {
        card.supported_interfaces
            .iter()
            .find(|interface| interface.protocol_version == A2A_VERSION)
    }

    /// Recommend a security scheme from those the card declares. The 1.0 card
    /// wraps each scheme in a proto-oneof (`httpAuthSecurityScheme`,
    /// `oauth2SecurityScheme`, `apiKeySecurityScheme`); a bearer/oauth2
    /// scheme is preferred for delegated access.
    fn recommend_auth_scheme(card: &AgentCard) -> String {
        let pick = card.security_schemes.iter().find(|(_, scheme)| {
            scheme.get("httpAuthSecurityScheme").is_some()
                || scheme.get("oauth2SecurityScheme").is_some()
        });

        match pick {
            Some((name, _)) => {
                info!("💡 Recommendation: authenticate with '{name}' (bearer/oauth2)");
                name.clone()
            }
            None => match card.security_schemes.keys().next() {
                Some(name) => {
                    info!("💡 Recommendation: authenticate with '{name}'");
                    name.clone()
                }
                None => {
                    warn!("⚠️  Card declares no security schemes");
                    "none".to_owned()
                }
            },
        }
    }

    /// Demonstrate the full discovery + negotiation flow.
    async fn demonstrate_capability_negotiation(&self) -> Result<()> {
        info!("\n🤝 Starting A2A 1.0 Discovery & Negotiation Demo\n");

        // Step 1: Fetch the peer's card.
        let agent_card = self.fetch_agent_card().await?;

        // Step 2: Analyze what it offers.
        Self::analyze_capabilities(&agent_card);

        // Step 3: Check for specific skills we need.
        info!("\n🔍 Skill Check:");
        let required_skills = ["get_activities", "analyze_activity", "set_goal"];
        for skill_id in required_skills {
            if Self::has_skill(&agent_card, skill_id) {
                info!("   ✅ Offers skill: {skill_id}");
            } else {
                info!("   ❌ Missing skill: {skill_id}");
            }
        }

        // Step 4: Find fitness-related skills.
        info!("\n🔎 Finding fitness-related skills:");
        let fitness_skills = Self::find_skills(&agent_card, "fitness");
        for skill in &fitness_skills {
            info!("   • {} - {}", skill.id, skill.description);
        }

        // Step 5: Negotiate transport.
        info!("\n🔌 Transport Negotiation:");
        let Some(interface) = Self::negotiate_interface(&agent_card) else {
            anyhow::bail!(
                "No interface serves A2A {A2A_VERSION}; this agent cannot collaborate with the peer"
            );
        };
        info!(
            "   ✅ Will use {} at {} (send `A2A-Version: {A2A_VERSION}` on every request)",
            interface.protocol_binding, interface.url
        );

        // Step 6: Recommend authentication.
        info!("\n🔐 Authentication Scheme Recommendation:");
        let _auth_scheme = Self::recommend_auth_scheme(&agent_card);

        // Step 7: Suitability decision.
        info!("\n✅ Agent Suitability Assessment:");
        if Self::has_skill(&agent_card, "get_activities") {
            info!("   ✅ This agent is suitable for fitness data analysis tasks");
            info!("   ✅ Offers {} fitness-related skills", fitness_skills.len());
            info!("   ✅ Recommended for integration");
        } else {
            info!("   ❌ This agent may not be suitable for fitness tasks");
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("agent_discovery_example=info")
        .init();

    info!("🚀 A2A 1.0 Agent Discovery Example");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Get server URL from environment or use default
    let server_url = std::env::var("PIERRE_SERVER_URL")
        .unwrap_or_else(|_| "http://localhost:8081".to_string());

    // Create discovery client
    let discovery = AgentDiscovery::new(server_url);

    // Run the demonstration
    match discovery.demonstrate_capability_negotiation().await {
        Ok(()) => {
            info!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("✅ Agent Discovery Demo Completed Successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            tracing::error!("❌ Agent Discovery Demo Failed: {}", e);
            tracing::error!("   Make sure Pierre server is running: cargo run --bin pierre-mcp-server");
            Err(e)
        }
    }
}
