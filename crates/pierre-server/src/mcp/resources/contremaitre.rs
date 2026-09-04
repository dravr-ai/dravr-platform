// ABOUTME: Contremaitre registry bootstrap + full-sync helper for ServerContext startup
// ABOUTME: Builds prompt/tool-desc/evidence/messaging-strings/training-catalogue registries and kicks off the initial GitHub/GCS overlay sync in the background
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::sync::{full_sync, ContremaitreRegistries};
use pierre_contremaitre::{
    ContremaitreConfig, EvidenceRegistry, MessagingStringsRegistry, PromptRegistry,
    ToolDescriptionRegistry, TrainingCatalogueRegistry,
};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{interval, sleep};
use tracing::{error, info, warn};

/// Default interval for the background prompt-reload poll.
///
/// The push webhook delivers a contremaitre change instantly to whichever
/// instance it lands on. This poll is the fan-out + catch-up path: every
/// running instance (and any that missed a webhook, or scaled up after the
/// push) independently re-converges on contremaitre HEAD. Each tick is a
/// single manifest read when nothing changed — `full_sync` compares the
/// manifest sha256 against the loaded registry sha and skips unchanged files
/// *before* fetching them — so it is negligible against the GitHub API budget.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;

/// Resolve the poll interval from `CONTREMAITRE_POLL_INTERVAL_SECS`, falling
/// back to [`DEFAULT_POLL_INTERVAL_SECS`]. A zero or unparseable value uses
/// the default rather than busy-looping.
fn poll_interval_secs() -> u64 {
    env::var("CONTREMAITRE_POLL_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
}

/// The live registries the Contremaitre sync overlays — `Arc` handles to the
/// same instances `ServerContext` serves from, so every tick lands the latest
/// contremaitre content where the chat pipeline reads it. Owned so the poll
/// task holds the set across ticks and clones it per respawn; borrowed into
/// the sync engine's [`ContremaitreRegistries`] shape at each tick.
#[derive(Clone)]
struct ContremaitreLiveRegistries {
    prompt: Arc<PromptRegistry>,
    tool_desc: Arc<ToolDescriptionRegistry>,
    evidence: Arc<EvidenceRegistry>,
    cageux_config: Arc<CageuxConfigRegistry>,
    messaging_strings: Arc<MessagingStringsRegistry>,
    persona_contract: Arc<PersonaContractRegistry>,
    training_catalogue: Arc<TrainingCatalogueRegistry>,
}

impl ContremaitreLiveRegistries {
    /// Borrow the set in the shape the sync engine takes.
    fn as_sync_registries(&self) -> ContremaitreRegistries<'_> {
        ContremaitreRegistries {
            registry: &self.prompt,
            tool_desc_registry: &self.tool_desc,
            evidence_registry: &self.evidence,
            cageux_config_registry: &self.cageux_config,
            messaging_strings_registry: &self.messaging_strings,
            persona_contract_registry: &self.persona_contract,
            training_catalogue_registry: &self.training_catalogue,
        }
    }
}

/// Run a contremaitre full-sync against the live registries, logging the
/// active backend (gcs vs github) and the result/error.
///
/// Shared by every tick of the background poll — including its first,
/// immediate tick, which is the initial startup overlay. Kept as its own
/// function so the poll closure stays under the workspace's
/// cognitive-complexity threshold; the body is an `if-let` plus `match`,
/// which clippy counts as two arms each.
async fn run_contremaitre_full_sync(
    config: &ContremaitreConfig,
    registries: &ContremaitreLiveRegistries,
) {
    let store = config.store();
    info!(
        backend = store.backend_label(),
        "Contremaitre sync starting"
    );
    let outcome = full_sync(registries.as_sync_registries(), store.as_ref()).await;
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

/// Spawn the background poll that runs [`full_sync`] on an interval.
///
/// Its first tick (immediate) is the initial startup overlay — run off the
/// bind path so a slow fetch never blocks the server from listening. Every
/// tick after fans prompt changes out to this instance (and recovers any
/// instance that missed the push webhook) without a redeploy. Owns its config
/// and the live registry bundle, so each tick overlays the latest
/// contremaitre content into the registries the chat pipeline reads.
fn spawn_contremaitre_poll(config: ContremaitreConfig, registries: ContremaitreLiveRegistries) {
    let secs = poll_interval_secs();
    info!(
        interval_secs = secs,
        "Contremaitre prompt-reload poll started"
    );
    // Supervisor: the poll loop runs in its own task whose JoinHandle is
    // awaited here, so a panicking tick is logged loudly and the loop is
    // respawned after one interval — a dead poll would otherwise freeze every
    // registry on its last-synced content for the life of the instance with
    // only a stderr panic as trace.
    tokio::spawn(async move {
        loop {
            let poll_config = config.clone();
            let poll_registries = registries.clone();
            let poll = tokio::spawn(async move {
                let mut ticker = interval(Duration::from_secs(secs));
                // `interval`'s first tick fires immediately, so the initial
                // overlay sync runs here — in the background, off the server's
                // bind path — and re-runs every `secs` after. Binding no longer
                // waits on a slow GitHub/GCS fetch, so a single-vCPU instance
                // still passes the Cloud Run startup probe.
                loop {
                    ticker.tick().await;
                    run_contremaitre_full_sync(&poll_config, &poll_registries).await;
                }
            });
            // The inner loop never returns normally; reaching here means the
            // task died (panic or runtime-cancelled join).
            if let Err(e) = poll.await {
                error!(
                    error = %e,
                    is_panic = e.is_panic(),
                    "Contremaitre poll loop died — respawning after one interval"
                );
            }
            sleep(Duration::from_secs(secs)).await;
        }
    });
}

/// The registries [`init_contremaitre_registries`] seeds and hands to
/// `ServerContext` — every one the sync overlays except the cageux config
/// and persona contract registries, which the caller builds first and
/// passes in.
pub(super) struct ContremaitreRegistrySet {
    pub prompt: Arc<PromptRegistry>,
    pub tool_desc: Arc<ToolDescriptionRegistry>,
    pub evidence: Arc<EvidenceRegistry>,
    pub messaging_strings: Arc<MessagingStringsRegistry>,
    pub training_catalogue: Arc<TrainingCatalogueRegistry>,
}

/// Build the prompt, tool-description, evidence, messaging-strings and
/// training-catalogue registries, and kick off the contremaitre overlay sync
/// in the background when configured.
///
/// The registries are returned immediately holding their compiled-in
/// defaults; the spawned poll's first (immediate) tick overlays the
/// GitHub/GCS content shortly after — off the server's bind path, so a slow
/// sync never delays startup (which would otherwise fail the Cloud Run
/// startup probe on a single vCPU). The cageux config registry is passed in
/// separately so the cageux snapshot exists whether or not the `contremaitre`
/// feature is enabled; when contremaitre IS enabled, its sync also populates
/// the cageux registry via the manifest's `config.cageux` entry.
pub(super) fn init_contremaitre_registries(
    cageux_config_registry: &Arc<CageuxConfigRegistry>,
    persona_contract_registry: &Arc<PersonaContractRegistry>,
) -> ContremaitreRegistrySet {
    let prompt_registry = Arc::new(PromptRegistry::new());
    let tool_desc_registry = Arc::new(ToolDescriptionRegistry::new());
    let evidence_registry = Arc::new(EvidenceRegistry::new());
    let messaging_strings_registry = Arc::new(MessagingStringsRegistry::new());
    let training_catalogue_registry = Arc::new(TrainingCatalogueRegistry::new());

    if let Some(config) = ContremaitreConfig::from_env() {
        // The poll owns the config and Arc clones of the live registries; its
        // first immediate tick is the initial overlay (background, off the
        // bind path), and every tick after is the fan-out / catch-up that
        // keeps each instance converged on contremaitre HEAD without a
        // redeploy, complementing the instant push webhook.
        spawn_contremaitre_poll(
            config,
            ContremaitreLiveRegistries {
                prompt: Arc::clone(&prompt_registry),
                tool_desc: Arc::clone(&tool_desc_registry),
                evidence: Arc::clone(&evidence_registry),
                cageux_config: Arc::clone(cageux_config_registry),
                messaging_strings: Arc::clone(&messaging_strings_registry),
                persona_contract: Arc::clone(persona_contract_registry),
                training_catalogue: Arc::clone(&training_catalogue_registry),
            },
        );
    } else {
        info!("Contremaitre not configured, using compiled-in defaults");
    }

    ContremaitreRegistrySet {
        prompt: prompt_registry,
        tool_desc: tool_desc_registry,
        evidence: evidence_registry,
        messaging_strings: messaging_strings_registry,
        training_catalogue: training_catalogue_registry,
    }
}
