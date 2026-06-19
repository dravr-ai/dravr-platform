// ABOUTME: Logging configuration and structured logging setup for observability and debugging
// ABOUTME: Configures log levels, formatters, and output destinations for comprehensive system logging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Production-ready logging configuration with structured output

/// Tenant-aware logging utilities and context management
pub mod tenant;

/// Google Cloud Logging structured-JSON formatter
pub mod gcp;

/// Span-field storage layer feeding ancestor-span context to formatters
pub mod span_fields;

/// OpenTelemetry OTLP pipeline (traces + metrics), compiled only with the
/// `telemetry` feature and dormant until `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
#[cfg(feature = "telemetry")]
pub mod otel;

/// Re-export tenant logging utilities
pub use tenant::{
    record_performance_metrics, record_request_context, record_tenant_context, ProviderApiContext,
    TenantLogger,
};

use gcp::GcpFormatter;
use span_fields::SpanFieldStorage;

use dravr_tronc::notifications::{
    EmailClient, ErrorNotificationLayer, NotificationConfig, PostHogClient, SlackClient,
    SlackConfig,
};
use dravr_tronc::notify::{AnalyticsProvider, NotifyEnricher, NotifyLayer};
use pierre_contremaitre::notify_routing::{ContremaitreRoutingProvider, NOTIFY_ROUTING_PROVIDER};
use pierre_core::constants::service_names;
use pierre_core::errors::AppResult;
use serde_json::json;
use std::env;
use std::fmt::Debug;
use std::io;
use std::sync::{Arc, OnceLock};
use tracing::field::{Field, Visit};
use tracing::{info, Event, Metadata, Subscriber};
use tracing_subscriber::{
    filter::{filter_fn, FilterFn},
    fmt::{self, format::FmtSpan},
    layer::{Context, Filter, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::Tracer;

/// Per-event closure type for the chromiumoxide teardown suppression
/// filter. Encoded as a `fn(&Metadata) -> bool` so it implements `Clone`
/// and `Copy` and can be wrapped in `FilterFn` once and applied to every
/// downstream sink (stdout formatter + tronc Slack notifier) without
/// fighting `tracing_subscriber`'s opaque-`Filter<S>` typing.
type TeardownFilterFn = fn(&tracing::Metadata<'_>) -> bool;

/// Returns `false` to suppress the event when:
///   * its target is `chromiumoxide::handler`, AND
///   * its level is ERROR, AND
///   * sciotte advertises a browser teardown in progress.
///
/// chromiumoxide drives Chrome over a DevTools-Protocol WebSocket.
/// Closing the browser exits Chrome without a polite WS Close frame,
/// so the chromiumoxide handler task observes a `Ws(Protocol(
/// ResetWithoutClosingHandshake))` and emits an `error!` log. The
/// scrape data was already collected before the close — but
/// `dravr-tronc` forwards every `error!` line to Slack, drowning real
/// failures in lifecycle noise.
///
/// Globally downgrading `chromiumoxide::handler` would also hide
/// genuine WS faults (Chrome crash mid-scrape, network reset on a
/// live session), so this predicate ONLY fires while sciotte's
/// `TeardownGuard` is alive (or in its 500 ms grace window). Outside
/// that window every chromiumoxide error reaches both Cloud Logging
/// and the Slack alert path with full fidelity.
fn chromiumoxide_teardown_predicate(metadata: &tracing::Metadata<'_>) -> bool {
    if metadata.target() == "chromiumoxide::handler"
        && *metadata.level() == tracing::Level::ERROR
        && dravr_sciotte::is_browser_teardown_in_progress()
    {
        return false;
    }
    true
}

/// Build a fresh `FilterFn` instance over [`chromiumoxide_teardown_predicate`].
/// Cheap (no allocation) — call once per layer, hand the result to
/// `.with_filter(...)`.
fn chromiumoxide_teardown_filter() -> FilterFn<TeardownFilterFn> {
    filter_fn(chromiumoxide_teardown_predicate as TeardownFilterFn)
}

/// Substring that identifies the chromiumoxide post-close WS reset pattern.
/// Matches both the tungstenite error `Display` (`Connection reset
/// without closing handshake`) and the `Debug` form chromiumoxide
/// emits (`Ws(Protocol(ResetWithoutClosingHandshake))`).
const CHROMIUMOXIDE_RESET_TOKEN: &str = "ResetWithoutClosingHandshake";

/// Visitor that records whether the event's `message` field contains a
/// given substring. Only inspects the `message` field — other fields are
/// ignored so we don't trip on incidental references in structured args.
struct MessageContainsVisitor<'a> {
    needle: &'a str,
    matched: bool,
}

impl<'a> MessageContainsVisitor<'a> {
    fn new(needle: &'a str) -> Self {
        Self {
            needle,
            matched: false,
        }
    }
}

impl Visit for MessageContainsVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            let rendered = format!("{value:?}");
            if rendered.contains(self.needle) {
                self.matched = true;
            }
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" && value.contains(self.needle) {
            self.matched = true;
        }
    }
}

/// Process-wide `AnalyticsProvider` for the `NotifyLayer` `PostHog` sink.
///
/// Set once at startup (before [`init_from_env`]) by the binary, which owns
/// the crates needed to build the provider (consent cache, tier catalogue,
/// identifier hashing). [`build_notify_layer`](LoggingConfig::build_notify_layer)
/// reads it and, when `POSTHOG_API_KEY` is also set, attaches the sink. Mirrors
/// the existing `NOTIFY_ROUTING_PROVIDER` injection pattern.
static ANALYTICS_PROVIDER: OnceLock<Arc<dyn AnalyticsProvider>> = OnceLock::new();

/// Register the `AnalyticsProvider` used by the `NotifyLayer` `PostHog` sink.
///
/// Call once before [`init_from_env`]. Subsequent calls are ignored (the slot
/// is set-once), so a re-init in tests can't clobber the live provider.
pub fn set_analytics_provider(provider: Arc<dyn AnalyticsProvider>) {
    let _ = ANALYTICS_PROVIDER.set(provider);
}

/// Process-wide [`NotifyEnricher`] for the `NotifyLayer`.
///
/// Set once at startup (before [`init_from_env`]) by the binary. The enricher
/// injects host-derived fields the call sites don't carry — a `user_email`
/// resolved from the in-process identity cache and a per-event display `emoji`
/// — into every notify event before it reaches Slack and `PostHog`. Applies to
/// both sink configurations (with or without the `PostHog` analytics sink), so
/// Slack identity rendering does not depend on `POSTHOG_API_KEY`.
static NOTIFY_ENRICHER: OnceLock<Arc<dyn NotifyEnricher>> = OnceLock::new();

/// Register the [`NotifyEnricher`] used by the `NotifyLayer`.
///
/// Call once before [`init_from_env`]. Subsequent calls are ignored (set-once).
pub fn set_notify_enricher(enricher: Arc<dyn NotifyEnricher>) {
    let _ = NOTIFY_ENRICHER.set(enricher);
}

/// Per-event filter applied to the Slack notification sink only.
///
/// Suppresses chromiumoxide handler ERROR events whose message contains
/// `ResetWithoutClosingHandshake` — that exact tungstenite protocol
/// variant fires whenever Chrome exits without sending a Close frame.
/// We see it in two paths: the explicit `close_browsers` route (already
/// covered by [`chromiumoxide_teardown_predicate`]'s grace window) and
/// the implicit `Drop` route when a `Browser` Arc is reaped without
/// going through `close_browsers`. Both cases are post-close lifecycle
/// noise — Chrome is gone, no operator action will recover it, and the
/// scrape function's own error path is the actionable signal.
///
/// Other chromiumoxide handler errors (TLS faults, capacity exhaustion,
/// distinct protocol violations) still page through the notifier with
/// full fidelity. Cloud Logging keeps every event regardless: this
/// filter only suppresses the *Slack ping*, not the structured log.
///
/// Exposed for integration tests so the filter can be wired into a
/// scratch subscriber without going through [`LoggingConfig::init`]
/// (which installs a process-global subscriber).
pub struct ChromiumoxideNotifierFilter;

impl<S> Filter<S> for ChromiumoxideNotifierFilter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, _metadata: &Metadata<'_>, _ctx: &Context<'_, S>) -> bool {
        // Defer the decision to `event_enabled` so we can inspect the
        // message field, which `Metadata` alone does not expose.
        true
    }

    fn event_enabled(&self, event: &Event<'_>, _ctx: &Context<'_, S>) -> bool {
        let meta = event.metadata();
        if meta.target() != "chromiumoxide::handler" || *meta.level() != tracing::Level::ERROR {
            return true;
        }
        let mut visitor = MessageContainsVisitor::new(CHROMIUMOXIDE_RESET_TOKEN);
        event.record(&mut visitor);
        !visitor.matched
    }
}

/// Log output options for controlling what information is included
#[derive(Debug, Clone, Copy)]
pub struct LogOutputOptions {
    /// Include source file and line numbers
    pub location: bool,
    /// Include thread information
    pub thread: bool,
    /// Include span information for tracing
    pub spans: bool,
}

/// Feature flags for optional logging capabilities
#[derive(Debug, Clone, Copy)]
pub struct LogFeatures {
    /// Enable OpenTelemetry tracing
    pub telemetry: bool,
    /// Truncate long MCP request/response logs for readability
    pub truncate_mcp: bool,
}

/// Logging configuration
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Log level (trace, debug, info, warn, error)
    pub level: String,
    /// Output format (json, pretty, compact)
    pub format: LogFormat,
    /// Output options controlling included information
    pub output: LogOutputOptions,
    /// Service name for structured logging
    pub service_name: String,
    /// Service version
    pub service_version: String,
    /// Environment (development, staging, production)
    pub environment: String,
    /// Feature flags for optional capabilities
    pub features: LogFeatures,
    /// Request ID header name
    pub request_id_header: String,
}

/// Log output format options
#[derive(Debug, Clone)]
pub enum LogFormat {
    /// `JSON` format for production logging
    Json,
    /// Pretty format for development
    Pretty,
    /// Compact format for space-constrained environments
    Compact,
    /// Google Cloud Logging structured JSON (Cloud Run / Stackdriver)
    Gcp,
}

/// Whether the OTLP telemetry pipeline will activate at startup.
///
/// True only when the `telemetry` feature is compiled in **and**
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is set — the "off by default, enabled on
/// demand" contract. This is the single source of truth read by both
/// `LogFeatures.telemetry` and the pipeline builder.
#[must_use]
pub fn telemetry_enabled() -> bool {
    cfg!(feature = "telemetry")
        && env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok_and(|v| !v.is_empty())
}

/// Live OTLP pipeline handle, set once during [`LoggingConfig::init`] and
/// drained by [`shutdown_telemetry`] on graceful shutdown.
#[cfg(feature = "telemetry")]
static TELEMETRY_HANDLE: OnceLock<otel::TelemetryHandle> = OnceLock::new();

/// Flush and shut down the OTLP telemetry pipeline if it was activated.
///
/// No-op when the `telemetry` feature is disabled or telemetry never started.
/// Call this on the graceful-shutdown path so batch exporters drain their
/// final buffered spans and metrics instead of dropping them.
pub fn shutdown_telemetry() {
    #[cfg(feature = "telemetry")]
    if let Some(handle) = TELEMETRY_HANDLE.get() {
        handle.shutdown();
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: LogFormat::Pretty,
            output: LogOutputOptions {
                location: false,
                thread: false,
                spans: false,
            },
            service_name: service_names::PIERRE_MCP_SERVER.into(),
            service_version: env!("CARGO_PKG_VERSION").to_owned(),
            environment: "development".into(),
            features: LogFeatures {
                telemetry: telemetry_enabled(),
                truncate_mcp: true, // Default to readable logs
            },
            request_id_header: "x-request-id".into(),
        }
    }
}

impl LoggingConfig {
    /// Create logging configuration from environment variables
    #[must_use]
    pub fn from_env() -> Self {
        let level = env::var("RUST_LOG").unwrap_or_else(|_| "info".into());

        // Cloud Run sets `K_SERVICE` on every revision; this is the canonical
        // signal that we are running in the managed environment, so default
        // to the GCP-structured log format there unless overridden.
        let on_cloud_run = env::var("K_SERVICE").is_ok();

        let format = match env::var("LOG_FORMAT").as_deref() {
            Ok("json") => LogFormat::Json,
            Ok("compact") => LogFormat::Compact,
            Ok("gcp") => LogFormat::Gcp,
            Ok("pretty") => LogFormat::Pretty,
            _ if on_cloud_run => LogFormat::Gcp,
            _ => LogFormat::Pretty,
        };

        let environment = env::var("ENVIRONMENT")
            .or_else(|_| env::var("NODE_ENV"))
            .unwrap_or_else(|_| "development".into());

        // In production, use more detailed logging
        let is_production = environment == "production" || on_cloud_run;

        Self {
            level,
            format,
            output: LogOutputOptions {
                location: is_production || env::var("LOG_INCLUDE_LOCATION").is_ok(),
                thread: is_production || env::var("LOG_INCLUDE_THREAD").is_ok(),
                spans: is_production || env::var("LOG_INCLUDE_SPANS").is_ok(),
            },
            service_name: env::var("SERVICE_NAME")
                .unwrap_or_else(|_| service_names::PIERRE_MCP_SERVER.into()),
            service_version: env::var("SERVICE_VERSION")
                .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned()),
            environment,
            features: LogFeatures {
                // Off by default; activates only with the `telemetry` feature
                // compiled in AND OTEL_EXPORTER_OTLP_ENDPOINT set.
                telemetry: telemetry_enabled(),
                truncate_mcp: env::var("MCP_LOG_TRUNCATE")
                    .map_or(true, |v| v != "false" && v != "0"), // Default to true (truncated) unless explicitly disabled
            },
            request_id_header: env::var("REQUEST_ID_HEADER")
                .unwrap_or_else(|_| "x-request-id".into()),
        }
    }

    /// Build the OTLP pipeline (if active) and return its tracer for the
    /// subscriber layer. Reads `self.features.telemetry` (the off-by-default
    /// gate) and stores the live handle for `shutdown_telemetry`. A build
    /// failure is logged to stderr and degrades to logs-only rather than
    /// aborting startup. Runs before the subscriber is installed, so it does
    /// not use tracing macros.
    #[cfg(feature = "telemetry")]
    fn init_telemetry(&self) -> Option<Tracer> {
        if !self.features.telemetry {
            return None;
        }
        match otel::build_telemetry(self) {
            Ok(Some(handle)) => {
                let tracer = handle.tracer();
                // `set` only errors if a handle is already installed (double
                // init); the first pipeline stays authoritative.
                let _ = TELEMETRY_HANDLE.set(handle);
                Some(tracer)
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!("OpenTelemetry init failed, continuing without telemetry: {e}");
                None
            }
        }
    }

    /// Initialize the global tracing subscriber
    ///
    /// # Errors
    ///
    /// Returns an error if the tracing subscriber fails to initialize
    pub fn init(&self) -> AppResult<()> {
        // Create environment filter that always applies our noise reduction rules
        let env_filter = env::var("RUST_LOG")
            .map_or_else(
                |_| {
                    // Default filter
                    EnvFilter::new(&self.level)
                },
                |env_directive| {
                    // If RUST_LOG is set, use it as base but add our noise reduction
                    EnvFilter::new(&env_directive)
                },
            )
            // Always apply noise reduction regardless of RUST_LOG setting
            .add_directive(
                "hyper=warn"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::WARN.into()),
            )
            .add_directive(
                "hyper::proto=warn"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::WARN.into()),
            )
            .add_directive(
                "reqwest=warn"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::WARN.into()),
            )
            .add_directive(
                "sqlx=info"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::INFO.into()),
            )
            .add_directive(
                "sqlx::query=info"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::INFO.into()),
            )
            .add_directive(
                "axum::rejection=info"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::INFO.into()),
            )
            .add_directive(
                "tower_http=info"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::INFO.into()),
            )
            // Headless-Chrome WebSocket handler emits non-actionable WARNs
            // for every malformed frame from the browser side; silence below ERROR.
            .add_directive(
                "chromiumoxide=error"
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::ERROR.into()),
            )
            // Keep our application logs at desired level
            .add_directive(
                format!("pierre_mcp_server={}", self.level)
                    .parse()
                    .unwrap_or_else(|_| tracing::Level::INFO.into()),
            );

        // Create base registry. The span-field storage layer captures each
        // span's field values so formatters can propagate turn-scoped context
        // (user_id, tenant_id, conversation_id, …) onto every child event.
        let registry = tracing_subscriber::registry()
            .with(env_filter)
            .with(SpanFieldStorage);

        // Build optional error notification layer (active when SLACK_ERROR_CHANNEL is set)
        let error_layer =
            Self::build_error_notification_layer(&self.service_name, &self.environment);

        // Build optional NotifyLayer for `target: "notify"` business events.
        // Returns `None` when SLACK_BOT_TOKEN is unset (local dev), which
        // makes the layer a no-op via Option<Layer> passthrough — the
        // events still land in Cloud Logging through the formatter layer.
        let notify_layer = Self::build_notify_layer(&self.environment);

        // Build the OTLP pipeline once (if active) and reuse its tracer across
        // the format branches. `Option<Tracer>` is subscriber-agnostic, so the
        // actual `OpenTelemetryLayer<S, _>` is constructed inside each branch
        // where `S` is known. `None` (telemetry off) makes the layer a no-op.
        #[cfg(feature = "telemetry")]
        let otel_tracer = self.init_telemetry();

        // Per-layer filter that suppresses chromiumoxide's expected
        // post-close WS-reset error events during the short teardown
        // window sciotte advertises. Built fresh per sink so both the
        // stdout/GCP-JSON formatter and the Slack error-notifier share
        // the same suppression logic; outside the teardown window
        // every event flows through normally.
        match self.format {
            LogFormat::Json => {
                let json_layer = fmt::layer()
                    .with_file(self.output.location)
                    .with_line_number(self.output.location)
                    .with_thread_ids(self.output.thread)
                    .with_thread_names(self.output.thread)
                    .with_target(true)
                    .with_writer(io::stdout)
                    .with_span_events(if self.output.spans {
                        FmtSpan::NEW | FmtSpan::CLOSE
                    } else {
                        FmtSpan::NONE
                    })
                    .json()
                    .with_filter(chromiumoxide_teardown_filter());

                let r = registry.with(json_layer).with(
                    error_layer
                        .with_filter(chromiumoxide_teardown_filter())
                        .with_filter(ChromiumoxideNotifierFilter),
                );
                let r = r.with(notify_layer);
                #[cfg(feature = "telemetry")]
                let r = r.with(otel_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)));
                r.init();
            }
            LogFormat::Pretty => {
                let pretty_layer = fmt::layer()
                    .with_file(self.output.location)
                    .with_line_number(self.output.location)
                    .with_thread_ids(self.output.thread)
                    .with_thread_names(self.output.thread)
                    .with_target(true)
                    .with_writer(io::stdout)
                    .with_span_events(if self.output.spans {
                        FmtSpan::NEW | FmtSpan::CLOSE
                    } else {
                        FmtSpan::NONE
                    })
                    .with_filter(chromiumoxide_teardown_filter());

                let r = registry.with(pretty_layer).with(
                    error_layer
                        .with_filter(chromiumoxide_teardown_filter())
                        .with_filter(ChromiumoxideNotifierFilter),
                );
                let r = r.with(notify_layer);
                #[cfg(feature = "telemetry")]
                let r = r.with(otel_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)));
                r.init();
            }
            LogFormat::Compact => {
                let compact_layer = fmt::layer()
                    .compact()
                    .with_file(false)
                    .with_line_number(false)
                    .with_thread_ids(false)
                    .with_thread_names(false)
                    .with_target(false)
                    .with_writer(io::stdout)
                    .with_span_events(FmtSpan::NONE)
                    .with_filter(chromiumoxide_teardown_filter());

                let r = registry.with(compact_layer).with(
                    error_layer
                        .with_filter(chromiumoxide_teardown_filter())
                        .with_filter(ChromiumoxideNotifierFilter),
                );
                let r = r.with(notify_layer);
                #[cfg(feature = "telemetry")]
                let r = r.with(otel_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)));
                r.init();
            }
            LogFormat::Gcp => {
                let gcp_layer = fmt::layer()
                    .with_writer(io::stdout)
                    .event_format(GcpFormatter::new(
                        &self.service_name,
                        &self.service_version,
                        &self.environment,
                        self.output.location,
                    ))
                    .with_filter(chromiumoxide_teardown_filter());

                let r = registry.with(gcp_layer).with(
                    error_layer
                        .with_filter(chromiumoxide_teardown_filter())
                        .with_filter(ChromiumoxideNotifierFilter),
                );
                let r = r.with(notify_layer);
                #[cfg(feature = "telemetry")]
                let r = r.with(otel_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)));
                r.init();
            }
        }

        // Log startup information
        self.log_startup_info();

        Ok(())
    }

    /// Log structured startup information
    fn log_startup_info(&self) {
        info!(
            service.name = %self.service_name,
            service.version = %self.service_version,
            environment = %self.environment,
            log.level = %self.level,
            log.format = ?self.format,
            "Pierre MCP Server starting up"
        );

        // Log configuration summary
        let config_summary = json!({
            "service": {
                "name": self.service_name,
                "version": self.service_version,
                "environment": self.environment
            },
            "logging": {
                "level": self.level,
                "format": format!("{:?}", self.format),
                "features": {
                    "location": self.output.location,
                    "thread": self.output.thread,
                    "spans": self.output.spans
                }
            }
        });

        info!("Configuration loaded: {}", config_summary);
    }

    /// Build the error notification layer from environment configuration
    ///
    /// Returns `Some(layer)` when `SLACK_ERROR_CHANNEL` (and `SLACK_BOT_TOKEN`) or
    /// email env vars are configured. Returns `None` otherwise, which is transparent
    /// to the tracing subscriber (Option<Layer> implements Layer as a passthrough).
    fn build_error_notification_layer(
        service_name: &str,
        environment: &str,
    ) -> Option<ErrorNotificationLayer> {
        let mut config = NotificationConfig::from_env();
        service_name.clone_into(&mut config.service_name);
        environment.clone_into(&mut config.environment);

        let slack = config.slack.as_ref().map(SlackClient::new);
        let email = config.email.as_ref().and_then(|c| {
            EmailClient::new(c)
                .map_err(|e| tracing::warn!(error = %e, "Failed to initialize email notifier"))
                .ok()
        });

        if slack.is_none() && email.is_none() {
            return None;
        }

        info!(
            slack = slack.is_some(),
            email = email.is_some(),
            "Error notification layer enabled"
        );

        Some(ErrorNotificationLayer::new(config, slack, email))
    }

    /// Build the `NotifyLayer` for `target: "notify"` business events.
    ///
    /// Returns `Some(layer)` when `SLACK_BOT_TOKEN` is set so the layer
    /// can actually post to Slack. Otherwise returns `None` — every
    /// notify-target event still lands in Cloud Logging via the
    /// formatter layer, but the Slack ping path is skipped. The routing
    /// rules come from the process-wide
    /// [`NOTIFY_ROUTING_PROVIDER`] which the contremaitre sync engine
    /// hot-reloads from `config/notify-routing.yaml`.
    ///
    /// `SLACK_ERROR_CHANNEL` is used as a placeholder default channel
    /// for the underlying `SlackClient` — `NotifyLayer` overrides it
    /// per event using the routing rule's `channel` field, so the
    /// placeholder is only relevant when the routing provider has no
    /// rule for the emitted event (in which case the layer drops the
    /// event entirely per its `default_rule = None` configuration).
    fn build_notify_layer(environment: &str) -> Option<NotifyLayer<ContremaitreRoutingProvider>> {
        let bot_token = env::var("SLACK_BOT_TOKEN").ok().filter(|s| !s.is_empty())?;
        // Placeholder channel — `NotifyLayer` reads the per-event channel
        // from the routing rule on every dispatch, so this string is only
        // used to construct the `SlackClient` and never appears in a real
        // notify-event Slack post.
        let error_channel = env::var("SLACK_ERROR_CHANNEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "#dravr-events".to_owned());
        let signing_secret = env::var("SLACK_SIGNING_SECRET")
            .ok()
            .filter(|s| !s.is_empty());

        let slack_config = SlackConfig {
            bot_token,
            error_channel,
            signing_secret,
        };
        let slack_client = Arc::new(SlackClient::new(&slack_config));
        let provider = Arc::clone(&NOTIFY_ROUTING_PROVIDER);

        info!(
            environment = environment,
            "NotifyLayer enabled: target=notify business events route via contremaitre"
        );

        // Attach the PostHog analytics sink when a project key is set and a
        // provider was registered at startup. The sink captures every
        // catalogued event (tier/consent decided by the provider) independent
        // of the Slack routing rules. Analytics rides on this layer, so it
        // currently requires SLACK_BOT_TOKEN (checked above) for the layer to
        // exist at all.
        let posthog = env::var("POSTHOG_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())
            .map(
                |key| match env::var("POSTHOG_HOST").ok().filter(|s| !s.is_empty()) {
                    Some(host) => Arc::new(PostHogClient::with_host(key, host)),
                    None => Arc::new(PostHogClient::new(key)),
                },
            );

        let mut builder = NotifyLayer::builder(slack_client, provider, environment.to_owned());

        // The enricher (user_email + emoji) applies to every notify event for
        // both sinks, so it is wired regardless of whether PostHog is enabled.
        if let Some(enricher) = NOTIFY_ENRICHER.get().cloned() {
            builder = builder.with_enricher(enricher);
        }

        // Attach the PostHog analytics sink when a project key is set and a
        // provider was registered at startup. The sink captures every
        // catalogued event (tier/consent decided by the provider) independent
        // of the Slack routing rules.
        if let (Some(analytics), Some(posthog)) = (ANALYTICS_PROVIDER.get().cloned(), posthog) {
            info!("PostHog analytics sink enabled on NotifyLayer");
            builder = builder.with_analytics(analytics, posthog);
        }

        Some(builder.build())
    }

    /// Create GCP optimized logging configuration
    #[must_use]
    pub fn for_gcp_cloud_run() -> Self {
        Self {
            level: "info".into(),
            format: LogFormat::Gcp,
            output: LogOutputOptions {
                location: true,
                thread: false,
                spans: true,
            },
            service_name: service_names::PIERRE_MCP_SERVER.into(),
            service_version: env!("CARGO_PKG_VERSION").to_owned(),
            environment: "production".into(),
            features: LogFeatures {
                // Endpoint-driven, even on Cloud Run: traces export only when
                // OTEL_EXPORTER_OTLP_ENDPOINT is configured on the revision.
                telemetry: telemetry_enabled(),
                truncate_mcp: false, // Production wants full logs
            },
            request_id_header: "x-request-id".into(),
        }
    }
}

/// Initialize logging with default configuration
///
/// # Errors
///
/// Returns an error if logging initialization fails
pub fn init_default() -> AppResult<()> {
    LoggingConfig::default().init()
}

/// Initialize logging from environment
///
/// # Errors
///
/// Returns an error if logging initialization fails
pub fn init_from_env() -> AppResult<()> {
    LoggingConfig::from_env().init()
}
