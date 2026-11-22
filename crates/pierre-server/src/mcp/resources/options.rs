// ABOUTME: ServerContextOptions — optional initialization parameters for ServerContext::new
// ABOUTME: Carries RSA key size, JWKS manager handle, LLM/Chat providers, and extra tool registrations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::admin::jwks::JwksManager;
use pierre_core::billing::BillingProvider;
use pierre_llm::ChatProvider;
use pierre_llm::LlmProvider;
#[cfg(feature = "client-messaging")]
use pierre_services::channel_adapters::ChannelAdapterFactory;
use pierre_services::lab_document_model::LabReaderModel;
use pierre_tool_runtime::RuntimeTool;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::services::archive_import::ArchiveImports;
use crate::services::turn_runner::TurnRunner;
use dravr_barrage::blob::BlobStore;
use dravr_barrage::store::ActivityRecordStore;

/// Optional initialization parameters for `ServerContext`
///
/// Used to pass optional configuration during server initialization without
/// exceeding function argument limits. Every field has a sensible default
/// but one: the channel adapter factory is named by whoever builds the
/// context, so there is no `Default` and no constructor that picks it.
pub struct ServerContextOptions {
    /// Size of RSA keys for JWT signing (2048 for tests, 4096 for production)
    pub rsa_key_size_bits: Option<usize>,
    /// Pre-existing JWKS manager (for test performance - reuses RSA keys)
    pub jwks_manager: Option<Arc<JwksManager>>,
    /// LLM provider for insight validation (injected for testing with mock providers)
    pub llm_provider: Option<Arc<dyn LlmProvider>>,
    /// Pre-built [`ChatProvider`] singleton shared by every chat / agent /
    /// social / memory-extraction / health-probe caller.
    ///
    /// The production binary builds this once at startup via
    /// [`ChatProvider::from_chain`] and passes it here so the same provider
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
    pub extra_tools: Vec<Arc<dyn RuntimeTool>>,
    /// Billing provider backing `/api/billing/*` and `/webhooks/{provider}`.
    ///
    /// The production binary wires a real provider (`StripeProvider`) when
    /// the Stripe environment is configured; otherwise this stays `None` and
    /// the resources init falls back to the in-tree `DummyProvider` so the
    /// platform compiles, tests run, and local dev works without a vendor.
    pub billing_provider: Option<Arc<dyn BillingProvider>>,
    /// Where messaging turns are started (registre#126).
    ///
    /// The production binary selects this from the environment — Cloud
    /// Tasks on GCP, so a turn runs inside a request Cloud Run waits for —
    /// and passes it here. Unset, turns run in-process through the in-flight
    /// tracker, which is what local stacks and tests want.
    pub turn_runner: Option<Arc<TurnRunner>>,
    /// Archive imports (carnet#1218): the binary passes the ones its
    /// database and environment select, a test its own stores and fetcher.
    /// Unset, imports are not available.
    pub archive_imports: Option<ArchiveImports>,
    /// The activity record store live provider reads are kept in: the binary
    /// passes the one its database selects. Unset, records are kept in
    /// memory, as a test wants; their projection is the cache either way.
    pub live_records: Option<Arc<dyn ActivityRecordStore>>,
    /// The deployment's blob store, which keeps uploaded files and an
    /// import's originals (`services::originals::blob_store_from_env`), or
    /// why there is none. Unset, uploads are kept in memory, as a test wants.
    pub blobs: Option<Result<Arc<dyn BlobStore>, String>>,
    /// The model the lab reader reads with: a test passes a scripted one.
    /// Unset, `LAB_READER_MODEL` chooses (unset there too, the rules read).
    pub lab_reader: Option<LabReaderModel>,
    /// The `pierre-pdf-text` binary an uploaded lab report is read with: a
    /// test passes the one cargo built for it. Unset, the one beside this
    /// executable (`services::lab_documents::pdf_text_bin_beside_server`).
    pub pdf_text_bin: Option<PathBuf>,
    /// What builds the channel adapter every message of this context is
    /// verified or sent through: the webhook routes, the retry and resume
    /// workers, and the proactive senders (carnet#1600). The server binary
    /// names [`pierre_services::channel_adapters::ConfigChannelAdapters`],
    /// the adapter the tenant's stored config names. A test names a factory
    /// whose sends stay in the process. It is not optional: a context that
    /// fell back to the production factory when a test forgot the field
    /// would post that test's fixtures to the channel's real host.
    ///
    /// The notification sink, the approval notice and the consent ask send
    /// through it too, and the linking route asks it where a pairing code's
    /// destination is looked up. The Discord Gateway and Slack Socket Mode
    /// clients do not go through it: only the server binary starts them, and
    /// each builds the adapter its turns reply through.
    #[cfg(feature = "client-messaging")]
    pub channel_adapters: Arc<dyn ChannelAdapterFactory>,
}

impl fmt::Debug for ServerContextOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerContextOptions")
            .field("rsa_key_size_bits", &self.rsa_key_size_bits)
            .field("chat_provider", &self.chat_provider)
            .field("turn_runner", &self.turn_runner)
            .finish_non_exhaustive()
    }
}

impl ServerContextOptions {
    /// Create options for testing (2048-bit RSA keys for speed), on the
    /// channel adapter factory the caller names.
    ///
    /// The factory is an argument so that no test context gets one by
    /// omission: a suite passes its offline factory, and a fixture that
    /// routes no message says so where it names the production one.
    #[must_use]
    pub fn testing(
        #[cfg(feature = "client-messaging")] channel_adapters: Arc<dyn ChannelAdapterFactory>,
    ) -> Self {
        Self {
            rsa_key_size_bits: Some(2048),
            jwks_manager: None,
            llm_provider: None,
            chat_provider: None,
            extra_tools: Vec::new(),
            billing_provider: None,
            turn_runner: None,
            archive_imports: None,
            live_records: None,
            blobs: None,
            lab_reader: None,
            pdf_text_bin: None,
            #[cfg(feature = "client-messaging")]
            channel_adapters,
        }
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

    /// Set the billing provider. When unset, resources init falls back to
    /// the in-tree `DummyProvider`.
    #[must_use]
    pub fn with_billing_provider(mut self, provider: Arc<dyn BillingProvider>) -> Self {
        self.billing_provider = Some(provider);
        self
    }
}
