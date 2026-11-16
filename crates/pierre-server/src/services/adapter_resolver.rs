// ABOUTME: Resolves a tenant's messaging channel to an outbound adapter and its stored config
// ABOUTME: The seam every proactive sender (backfill, check-in, first-sync, group, commitments) shares

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use async_trait::async_trait;
use dravr_canot::channel::MessagingChannel;
use pierre_core::models::messaging::{ChannelConfig, ChannelType};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_database::RepositoryRegistry;
use pierre_services::channel_adapters::ChannelAdapterFactory;
use tracing::warn;

/// Resolves a `(tenant, channel)` to an outbound adapter + its config.
///
/// Extracted as a seam so the routing logic in
/// `ServerBackfillNotifier::push_backfill_complete` can be exercised against a
/// fake adapter that captures the [`OutgoingMessage`](pierre_core::models::messaging::OutgoingMessage) without touching a channel
/// API. The production [`ConfigAdapterResolver`] loads the tenant's stored
/// channel config and builds the real adapter exactly like the approval
/// notifier does.
#[async_trait]
pub trait AdapterResolver: Send + Sync {
    /// Resolve the channel adapter and its config for an outbound send, or
    /// `None` (logged) when the channel is unconfigured or undeserializable.
    async fn resolve(
        &self,
        tenant_id: TenantId,
        channel_str: &str,
        channel_type: ChannelType,
    ) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)>;
}

/// Production [`AdapterResolver`]: loads the tenant's stored channel config and
/// builds the real channel adapter from it.
///
/// Shared with the commitment reporter, which needs the same
/// `(tenant, channel) -> adapter + config` resolution and the same fake-adapter
/// test seam. Reach it through [`config_adapter_resolver`].
pub(crate) struct ConfigAdapterResolver {
    /// Shared repository registry — the channel-config lookup goes through
    /// `repos.messaging`. Arc so the resolver is cheap to share with the
    /// notifier and any future caller.
    pub(crate) repos: Arc<RepositoryRegistry>,
    /// Builds the adapter from the stored config: the context's own factory,
    /// so a proactive send leaves through what a webhook reply leaves through.
    pub(crate) adapters: Arc<dyn ChannelAdapterFactory>,
}

/// Build the production adapter resolver over the context's adapter factory.
pub(crate) fn config_adapter_resolver(
    repos: Arc<RepositoryRegistry>,
    adapters: Arc<dyn ChannelAdapterFactory>,
) -> Arc<dyn AdapterResolver> {
    Arc::new(ConfigAdapterResolver { repos, adapters })
}

#[async_trait]
impl AdapterResolver for ConfigAdapterResolver {
    async fn resolve(
        &self,
        tenant_id: TenantId,
        channel_str: &str,
        channel_type: ChannelType,
    ) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)> {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        let raw_config = match db.get_channel_config(tenant_id, channel_str).await {
            Ok(Some(cfg)) => cfg,
            Ok(None) => {
                warn!(channel = %channel_str, "No channel config for outbound notice");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "Failed to load channel config for outbound notice");
                return None;
            }
        };
        // The reason names the missing field, never a credential's value.
        let adapter = self
            .adapters
            .build(channel_type, &raw_config)
            .inspect_err(|e| {
                warn!(
                    error = %e,
                    channel = %channel_str,
                    tenant_id = %tenant_id,
                    "Failed to build adapter for outbound notice"
                );
            })
            .ok()?;
        let channel_config: ChannelConfig = serde_json::from_value(raw_config)
            .inspect_err(|e| {
                warn!(error = %e, "Failed to deserialize channel config for outbound notice");
            })
            .ok()?;
        Some((adapter, channel_config))
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::sync::Mutex;

    use chrono::Utc;
    use pierre_core::models::{Tenant, User};
    use pierre_database::backends::{TenantRepository, UpsertChannelConfigParams};
    use pierre_services::channel_adapters::ConfigChannelAdapters;
    use pierre_test_support::db::create_test_db;
    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;
    use uuid::Uuid;

    use super::*;

    /// A bot token no warn line may carry.
    const BOT_TOKEN: &str = "1600:never-in-a-log-line";

    /// Every warn event's fields, each rendered `name=value`.
    #[derive(Clone, Default)]
    struct Warns(Arc<Mutex<Vec<Vec<String>>>>);

    /// Collects one event's fields.
    #[derive(Default)]
    struct Fields(Vec<String>);

    impl Visit for Fields {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            self.0.push(format!("{}={value:?}", field.name()));
        }
    }

    impl<S: Subscriber> Layer<S> for Warns {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            if *event.metadata().level() == Level::WARN {
                let mut fields = Fields::default();
                event.record(&mut fields);
                self.0.lock().expect("warn log").push(fields.0);
            }
        }
    }

    /// A stored Telegram config that names a bot and no webhook secret, the
    /// credential the adapter is built on, under a tenant of its own.
    async fn tenant_with_an_unbuildable_telegram_config(repos: &RepositoryRegistry) -> TenantId {
        let owner = User::new(
            format!("{}@resolver.test", Uuid::new_v4()),
            String::new(),
            None,
        );
        let owner_user_id = repos.users.create(&owner).await.expect("user row");
        let tenant_id = TenantId::generate();
        let tenants: &dyn TenantRepository = repos.tenants.as_ref();
        tenants
            .create(&Tenant {
                id: tenant_id,
                name: "Resolver tenant".to_owned(),
                slug: format!("resolver-{tenant_id}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .expect("tenant row");
        let messaging: &dyn MessagingRepository = repos.messaging.as_ref();
        messaging
            .upsert_channel_config(&UpsertChannelConfigParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id,
                channel_type: "telegram",
                api_key: None,
                api_secret: None,
                webhook_secret: None,
                verify_token: None,
                account_id: None,
                phone_number: None,
                bot_token: Some(BOT_TOKEN),
                is_active: true,
            })
            .await
            .expect("channel config");
        tenant_id
    }

    /// An outbound notice that cannot be sent says why and for which tenant:
    /// the line an admin reads to tell a half-configured bot from a channel
    /// the build does not carry. It names no credential.
    #[tokio::test]
    async fn an_unbuildable_adapter_is_warned_with_its_reason_and_tenant() {
        let db = create_test_db().await.expect("test database");
        let repos = Arc::clone(db.repositories());
        let tenant_id = tenant_with_an_unbuildable_telegram_config(&repos).await;
        let resolver = config_adapter_resolver(repos, Arc::new(ConfigChannelAdapters));

        let warns = Warns::default();
        let capture = tracing_subscriber::registry()
            .with(warns.clone())
            .set_default();
        let outcome = resolver
            .resolve(tenant_id, "telegram", ChannelType::Telegram)
            .await;

        // The capture ends here: only the resolve above is read.
        drop(capture);

        assert!(outcome.is_none(), "no adapter, so nothing to send through");
        let warns = warns.0.lock().expect("warn log").clone();
        let [fields] = warns.as_slice() else {
            panic!("one warn for the one failed build, got {warns:?}");
        };
        assert!(
            fields
                .iter()
                .any(|f| f.starts_with("error=") && f.contains("missing webhook_secret")),
            "the warn carries why the adapter could not be built: {fields:?}"
        );
        assert!(
            fields.contains(&format!("tenant_id={tenant_id}")),
            "the warn names the tenant whose config it is: {fields:?}"
        );
        assert!(
            fields.contains(&"channel=telegram".to_owned()),
            "the warn names the channel: {fields:?}"
        );
        assert!(
            !fields.iter().any(|f| f.contains(BOT_TOKEN)),
            "the warn carries no credential: {fields:?}"
        );
    }
}
