# Agent Discovery Example (A2A 1.0)

Demonstrates A2A 1.0 **agent card discovery**, **interface negotiation**, and
**auth selection** — how one agent reads a peer's card before collaborating.

## What This Example Demonstrates

### A2A 1.0 Discovery Workflow:
1. **Fetch Agent Card** — retrieve the card from `/.well-known/agent-card.json` (RFC 8615)
2. **Analyze Card** — read `supportedInterfaces`, the `capabilities` object, and `skills`
3. **Skill Matching** — check whether the agent offers the skills you need
4. **Skill Discovery** — find relevant skills by id/description/tag
5. **Transport Negotiation** — pick the preferred `supportedInterface` that serves your protocol version
6. **Auth Selection** — choose a scheme from `securitySchemes`
7. **Suitability Assessment** — decide whether the agent fits the use case

## Why Agent Discovery Matters

In A2A, agents **discover each other's capabilities** before collaboration:

- **Avoid Assumptions** — don't assume what an agent can do; read its card
- **Transport Negotiation** — the card lists interfaces preference-ordered; pick the first you support
- **Version Compatibility** — each interface declares its `protocolVersion`; every request carries `A2A-Version`
- **Auth Negotiation** — select a `securityScheme` appropriate to the use case

## Quick Start

```bash
# 1. Start Pierre server (in another terminal)
cd ../../../
cargo run --bin pierre-mcp-server

# 2. Run the discovery example
cd examples/agents/agent_discovery
cargo run
```

## Example Output

```
🚀 A2A 1.0 Agent Discovery Example
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
📡 Fetching agent card from: http://localhost:8081/.well-known/agent-card.json
✅ Successfully fetched agent card for: Dravr AI

📊 Agent Capability Analysis:
   Agent: Dravr AI v1.0.0
   Description: AI-powered fitness data analysis and insights platform...

🔌 Transport Interfaces (2):
   • JSONRPC (A2A 1.0) at http://localhost:8081/a2a/jsonrpc [★ preferred]
   • HTTP+JSON (A2A 1.0) at http://localhost:8081/a2a [alternate]

⚙️  Protocol Capabilities:
   • streaming (SSE):        true
   • push notifications:     true
   • extended agent card:    true

🛠️  Available Skills (4):
   • get_activities (fitness, activities, data) - Retrieve user fitness activities from connected providers
   • analyze_activity (fitness, analysis, intelligence) - AI-powered analysis of a specific fitness activity
   • get_athlete (fitness, profile) - Retrieve athlete profile information
   • set_goal (fitness, goals) - Set a fitness goal for the user

🔐 Security Schemes:
   • bearerAuth
   • oauth2ClientCredentials

🔍 Skill Check:
   ✅ Offers skill: get_activities
   ✅ Offers skill: analyze_activity
   ✅ Offers skill: set_goal

🔎 Finding fitness-related skills:
   • get_activities - Retrieve user fitness activities from connected providers
   • analyze_activity - AI-powered analysis of a specific fitness activity
   ...

🔌 Transport Negotiation:
   ✅ Will use JSONRPC at http://localhost:8081/a2a/jsonrpc (send `A2A-Version: 1.0` on every request)

🔐 Authentication Scheme Recommendation:
💡 Recommendation: authenticate with 'bearerAuth' (bearer/oauth2)

✅ Agent Suitability Assessment:
   ✅ This agent is suitable for fitness data analysis tasks
   ✅ Offers 4 fitness-related skills
   ✅ Recommended for integration
```

## Key Concepts Demonstrated

### 1. Card fetching (RFC 8615 well-known path)
```rust
let url = format!("{}/.well-known/agent-card.json", self.server_url);
let agent_card = self.fetch_agent_card().await?;
```
Discovery is public and NOT version-gated — it is how a client learns which
protocol versions and interfaces the peer supports.

### 2. Skill checking
```rust
fn has_skill(card: &AgentCard, skill_id: &str) -> bool {
    card.skills.iter().any(|skill| skill.id == skill_id)
}
```
Before delegating, check whether the peer offers the skills you need.

### 3. Transport negotiation
```rust
// supportedInterfaces is preference-ordered; pick the first that serves our version.
let interface = card.supported_interfaces
    .iter()
    .find(|i| i.protocol_version == "1.0");
```
Interfaces are functionally equivalent; the client picks the first binding it
supports and sends `A2A-Version` on every request.

### 4. Auth selection
```rust
// securitySchemes are proto-oneof wrapped: httpAuthSecurityScheme / oauth2SecurityScheme / ...
fn recommend_auth_scheme(card: &AgentCard) -> String { /* prefer bearer/oauth2 */ }
```

## Real-World Use Cases

1. **Multi-Agent Systems** — before delegating, check the peer's skills
2. **Dynamic Agent Selection** — choose among agents by their advertised skills
3. **Transport Fallback** — if the preferred binding isn't supported, fall back to an alternate `supportedInterface`
4. **Version Compatibility** — reject a peer that serves no interface at your protocol version

## Integration with Other Examples

- **fitness_analyzer** — uses this discovery pattern before connecting to Pierre
- **task_manager** — discovers task capabilities before submitting long-running tasks

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `PIERRE_SERVER_URL` | `http://localhost:8081` | Pierre server base URL |

## A2A 1.0 Specification Compliance

This example demonstrates:

- ✅ Agent Card discovery at `/.well-known/agent-card.json`
- ✅ Card format (name, version, `supportedInterfaces`, `capabilities`, `skills`, `securitySchemes`)
- ✅ Transport negotiation (preference-ordered interfaces + `protocolVersion`)
- ✅ Auth selection from proto-oneof `securitySchemes`

## Learn More

- [A2A Protocol Specification](https://a2a-protocol.org/v1.0.0/specification)
- [Pierre A2A Documentation](../../../book/src/protocols.md)
- [Agent Card Design](../../../crates/pierre-a2a/src/agent_card.rs)
