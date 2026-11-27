// ABOUTME: The resource servers an authorization server mints for beyond the MCP one: A2A, agent-write and the satellites
// ABOUTME: Their builders, and every identifier the resource checks take, in one place
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::OAuth2AuthorizationServer;

impl OAuth2AuthorizationServer {
    /// Also mint for the A2A resource server, known by `a2a_resources`
    /// (`OAuth2ServerConfig::a2a_resources`): a grant bound to it never mints
    /// for the MCP resource server, or the reverse.
    #[must_use]
    pub fn with_a2a_resources(mut self, a2a_resources: Vec<String>) -> Self {
        self.a2a_resources = a2a_resources;
        self
    }

    /// Also mint for the agent-write resource, known by `resources`
    /// (`OAuth2ServerConfig::agent_write_resources`): only an agent-write
    /// grant bound by `DPoP` is bound to it, and it mints for nothing else.
    #[must_use]
    pub fn with_agent_write_resources(mut self, resources: Vec<String>) -> Self {
        self.agent_write_resources = resources;
        self
    }

    /// Also mint for the satellite MCP servers `satellites` names
    /// (`OAuth2ServerConfig::satellite_resources`), each its own resource
    /// server: a grant bound to one never mints for the platform or another.
    #[must_use]
    pub fn with_satellite_resources(mut self, satellites: Vec<String>) -> Self {
        self.satellites = satellites;
        self
    }

    /// Every identifier this server mints for, the platform's, then the
    /// agent-write resource's, then the satellites', as the resource checks
    /// take them.
    pub(super) fn served(&self) -> Vec<&str> {
        self.resources
            .iter()
            .chain(&self.a2a_resources)
            .chain(&self.agent_write_resources)
            .chain(&self.satellites)
            .map(String::as_str)
            .collect()
    }

    /// Whether `resource` is one of the agent-write resource's identifiers.
    pub(super) fn is_agent_write_resource(&self, resource: Option<&str>) -> bool {
        resource.is_some_and(|resource| {
            self.agent_write_resources
                .iter()
                .any(|served| served == resource)
        })
    }
}
