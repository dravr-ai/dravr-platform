// ABOUTME: ToolRuntime trait impl for ServerContext — narrow façade tools see during dispatch
// ABOUTME: Maps each ToolRuntime accessor to the corresponding Arc field or delegation method on ServerContext via the slice partition
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::ServerContext;
use pierre_auth::tenant::TenantOAuthClient;
use pierre_cache::Cache;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_database::backends::factory::Database;
use pierre_database::database::repositories::CoachesRepository;
use pierre_database::RepositoryRegistry;
#[cfg(feature = "health-sync")]
use pierre_enforme::SyncOrchestrator;
use pierre_intelligence::ActivityIntelligence;
use pierre_llm::LlmProvider;
use pierre_mcp_transport::sampling_peer::SamplingPeer;
#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;
use pierre_providers::registry::ProviderRegistry;
use pierre_runtime_context::DataContext;
#[cfg(feature = "transport-sse")]
use pierre_services::provider_refresh::SyncNotifier;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
use std::sync::Arc;

impl ToolRuntime for ServerContext {
    fn database(&self) -> &Arc<Database> {
        &self.coach.database
    }

    fn repos(&self) -> &Arc<RepositoryRegistry> {
        &self.common.repos
    }

    fn cache(&self) -> &Arc<Cache> {
        &self.common.cache
    }

    fn config(&self) -> &Arc<ServerConfig> {
        &self.common.config
    }

    fn provider_registry(&self) -> &Arc<ProviderRegistry> {
        &self.fitness.provider_registry
    }

    fn tenant_oauth_client(&self) -> &Arc<TenantOAuthClient> {
        &self.auth.tenant_oauth_client
    }

    fn cageux_config_registry(&self) -> &Arc<CageuxConfigRegistry> {
        &self.fitness.cageux_config_registry
    }

    fn activity_intelligence(&self) -> &Arc<ActivityIntelligence> {
        &self.fitness.activity_intelligence
    }

    fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.mcp.tool_registry
    }

    fn tool_selection(&self) -> &Arc<ToolSelectionService> {
        &self.mcp.tool_selection
    }

    fn coaches_manager(&self) -> &dyn CoachesRepository {
        Self::coaches_manager(self)
    }

    fn recommendation_system_prompt(&self) -> String {
        Self::recommendation_system_prompt(self)
    }

    fn recommendation_analysis_prompt(&self) -> String {
        Self::recommendation_analysis_prompt(self)
    }

    fn activity_analysis_prompt(&self) -> String {
        Self::activity_analysis_prompt(self)
    }

    fn activity_analysis_system_prompt(&self) -> String {
        Self::activity_analysis_system_prompt(self)
    }

    fn insight_generation_prompt(&self) -> String {
        Self::insight_generation_prompt(self)
    }

    fn insight_validation_prompt(&self) -> String {
        Self::insight_validation_prompt(self)
    }

    fn llm_provider(&self) -> Option<&Arc<dyn LlmProvider>> {
        self.common.llm_provider.as_ref()
    }

    fn evidence_registry(&self) -> &Arc<pierre_contremaitre::EvidenceRegistry> {
        &self.mcp.evidence_registry
    }

    fn data(&self) -> DataContext {
        Self::data(self)
    }

    fn sampling_peer(&self) -> Option<&Arc<SamplingPeer>> {
        self.sse.sampling_peer.as_ref()
    }

    #[cfg(feature = "client-notifications")]
    fn notification_service(&self) -> Option<&Arc<NotificationService>> {
        self.common.notification_service.as_ref()
    }

    #[cfg(feature = "transport-sse")]
    fn sync_notifier(&self) -> Arc<dyn SyncNotifier> {
        self.sse.sse_manager.clone()
    }

    #[cfg(feature = "health-sync")]
    fn sync_orchestrator(&self) -> Option<&Arc<SyncOrchestrator>> {
        self.fitness.sync_orchestrator.as_ref()
    }
}
