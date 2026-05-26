// ABOUTME: ServerContextOptions — optional initialization parameters for ServerContext::new
// ABOUTME: Carries RSA key size, JWKS manager handle, LLM/Chat providers, and extra tool registrations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::admin::jwks::JwksManager;
use pierre_llm::ChatProvider;
use pierre_llm::LlmProvider;
use pierre_tool_runtime::traits::McpTool;
use std::sync::Arc;

/// Optional initialization parameters for `ServerContext`
///
/// Used to pass optional configuration during server initialization without
/// exceeding function argument limits. All fields have sensible defaults.
#[derive(Default)]
pub struct ServerContextOptions {
    /// Size of RSA keys for JWT signing (2048 for tests, 4096 for production)
    pub rsa_key_size_bits: Option<usize>,
    /// Pre-existing JWKS manager (for test performance - reuses RSA keys)
    pub jwks_manager: Option<Arc<JwksManager>>,
    /// LLM provider for insight validation (injected for testing with mock providers)
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Pre-built [`ChatProvider`] singleton shared by every chat / coach /
    /// social / memory-extraction / health-probe caller.
    ///
    /// The production binary builds this once at startup via
    /// [`ChatProvider::from_env`] and passes it here so the same provider
    /// instance — and, for the Copilot Headless runner, the same long-lived
    /// `copilot --acp` subprocess + cached GitHub→Copilot OAuth token — is
    /// reused across every call. Without this, every chat request and every
    /// 5-minute health probe would re-invoke `from_env`, spawn a fresh
    /// subprocess, and re-run the token exchange.
    ///
    /// Tests that don't need a real provider leave this `None` and pass
    /// [`Self::llm_provider`] instead; the resources init wraps that in
    /// [`ChatProvider::Custom`] at construction time.
    pub chat_provider: Option<Arc<ChatProvider>>,
    /// Extra MCP tools to register in the default tool registry.
    ///
    /// Populated by messaging-eval integration tests that need a
    /// no-auth stub tool to exercise the tool-execution loop
    /// end-to-end without requiring a real provider connection.
    /// Production callers leave this empty — the default registry
    /// already holds every user-facing tool.
    pub extra_tools: Vec<Arc<dyn McpTool>>,
}

impl ServerContextOptions {
    /// Create options with production defaults (4096-bit RSA keys)
    #[must_use]
    pub fn production() -> Self {
        Self {
            rsa_key_size_bits: Some(4096),
            jwks_manager: None,
            llm_provider: None,
            chat_provider: None,
            extra_tools: Vec::new(),
        }
    }

    /// Create options for testing (2048-bit RSA keys for speed)
    #[must_use]
    pub fn testing() -> Self {
        Self {
            rsa_key_size_bits: Some(2048),
            jwks_manager: None,
            llm_provider: None,
            chat_provider: None,
            extra_tools: Vec::new(),
        }
    }

    /// Set the RSA key size
    #[must_use]
    pub const fn with_rsa_key_size(mut self, size: usize) -> Self {
        self.rsa_key_size_bits = Some(size);
        self
    }

    /// Set the JWKS manager
    #[must_use]
    pub fn with_jwks_manager(mut self, jwks: Arc<JwksManager>) -> Self {
        self.jwks_manager = Some(jwks);
        self
    }

    /// Set the LLM provider
    #[must_use]
    pub fn with_llm_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    /// Set the pre-built [`ChatProvider`] singleton.
    #[must_use]
    pub fn with_chat_provider(mut self, provider: Arc<ChatProvider>) -> Self {
        self.chat_provider = Some(provider);
        self
    }
}
