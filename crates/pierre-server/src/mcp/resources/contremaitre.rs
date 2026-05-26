// ABOUTME: Contremaitre registry bootstrap + full-sync helper for ServerContext startup
// ABOUTME: Builds prompt/tool-desc/evidence/messaging-strings registries and runs the initial GitHub/GCS overlay sync
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "contremaitre")]

use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::sync::full_sync;
use pierre_contremaitre::{
    ContremaitreConfig, EvidenceRegistry, MessagingStringsRegistry, PromptRegistry,
    ToolDescriptionRegistry,
};
use std::sync::Arc;
use tracing::{info, warn};

/// Bundle of registries pushed to the Contremaitre sync. Lives at module
/// scope so [`run_contremaitre_full_sync`] doesn't need a sprawling
/// positional argument list — every consumer threads the same registries.
pub(super) struct ContremaitreSyncRegistries<'a> {
    pub prompt: &'a Arc<PromptRegistry>,
    pub tool_desc: &'a Arc<ToolDescriptionRegistry>,
    pub evidence: &'a Arc<EvidenceRegistry>,
    pub cageux_config: &'a Arc<CageuxConfigRegistry>,
    pub messaging_strings: &'a Arc<MessagingStringsRegistry>,
    pub persona_contract: &'a Arc<PersonaContractRegistry>,
}

/// Run a contremaitre full-sync against the freshly-built registries,
/// logging the active backend (gcs vs github) and the result/error.
///
/// Extracted from [`init_contremaitre_registries`] to keep that function's
/// cognitive-complexity budget under the workspace's clippy threshold;
/// the block contains an `if-let` plus `match`, which clippy counts as
/// two arms each.
pub(super) async fn run_contremaitre_full_sync(
    config: &ContremaitreConfig,
    registries: ContremaitreSyncRegistries<'_>,
) {
    let store = config.store();
    info!(
        backend = store.backend_label(),
        "Contremaitre sync starting"
    );
    let outcome = full_sync(
        pierre_contremaitre::sync::ContremaitreRegistries {
            registry: registries.prompt,
            tool_desc_registry: registries.tool_desc,
            evidence_registry: registries.evidence,
            cageux_config_registry: registries.cageux_config,
            messaging_strings_registry: registries.messaging_strings,
            persona_contract_registry: registries.persona_contract,
        },
        store.as_ref(),
    )
    .await;
    match outcome {
        Ok(result) => info!(
            %result,
            backend = store.backend_label(),
            "Contremaitre sync complete"
        ),
        Err(e) => warn!(
            error = %e,
            backend = store.backend_label(),
            "Contremaitre sync failed, using compiled-in defaults"
        ),
    }
}

/// Initialize prompt, tool description, and evidence registries and sync
/// from contremaitre when configured.
///
/// The cageux config registry is passed in separately so that the cageux
/// snapshot exists whether or not the `contremaitre` feature is enabled.
/// When contremaitre IS enabled, its sync also populates the cageux
/// registry via the manifest's `config.cageux` entry.
pub(super) async fn init_contremaitre_registries(
    cageux_config_registry: &Arc<CageuxConfigRegistry>,
    persona_contract_registry: &Arc<PersonaContractRegistry>,
) -> (
    Arc<PromptRegistry>,
    Arc<ToolDescriptionRegistry>,
    Arc<EvidenceRegistry>,
    Arc<MessagingStringsRegistry>,
) {
    let prompt_registry = Arc::new(PromptRegistry::new());
    let tool_desc_registry = Arc::new(ToolDescriptionRegistry::new());
    let evidence_registry = Arc::new(EvidenceRegistry::new());
    let messaging_strings_registry = Arc::new(MessagingStringsRegistry::new());

    if let Some(config) = ContremaitreConfig::from_env() {
        run_contremaitre_full_sync(
            &config,
            ContremaitreSyncRegistries {
                prompt: &prompt_registry,
                tool_desc: &tool_desc_registry,
                evidence: &evidence_registry,
                cageux_config: cageux_config_registry,
                messaging_strings: &messaging_strings_registry,
                persona_contract: persona_contract_registry,
            },
        )
        .await;
    } else {
        info!("Contremaitre not configured, using compiled-in defaults");
    }

    (
        prompt_registry,
        tool_desc_registry,
        evidence_registry,
        messaging_strings_registry,
    )
}
