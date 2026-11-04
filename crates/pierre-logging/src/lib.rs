// ABOUTME: Logging configuration and structured logging setup for observability and debugging
// ABOUTME: Configures log levels, formatters, and output destinations for comprehensive system logging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Production-ready logging configuration with structured output

/// Records request identity on every enclosing span that declares it
pub mod tenant;

/// Google Cloud Logging structured-JSON formatter
pub mod gcp;

/// Span-field storage layer feeding ancestor-span context to formatters
pub mod span_fields;

/// The Cloud Trace id an HTTP request arrived with, for log correlation
pub mod request_trace;

/// OpenTelemetry OTLP pipeline (traces + metrics), compiled only with the
/// `telemetry` feature and dormant until `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
#[cfg(feature = "telemetry")]
pub mod otel;

/// Log-to-trace correlation for the GCP formatter, compiled with `telemetry`.
#[cfg(feature = "telemetry")]
mod trace_link;

/// Per-callsite throttle for repeated log lines
mod throttle;

/// Record identity (user, tenant) across the current span scope
pub use tenant::{record_identity, record_in_scope, TENANT_ID_FIELD, USER_ID_FIELD};

use gcp::GcpFormatter;
use span_fields::SpanFieldStorage;
use throttle::{CallsiteThrottle, DEFAULT_THROTTLE_BURST, THROTTLE_WINDOW};

use dravr_tronc::notifications::{
    EmailClient, ErrorNotificationLayer, NotificationConfig, PostHogClient, SlackClient,
    SlackConfig,
};
use dravr_tronc::notify::{
    AnalyticsProvider, NotifyActions, NotifyEnricher, NotifyLayer, NotifyLayerBuilder,
};
use dravr_tronc::secrets::{self, RedactingMakeWriter};
use pierre_contremaitre::notify_routing::{ContremaitreRoutingProvider, NOTIFY_ROUTING_PROVIDER};
use pierre_core::constants::service_names;
use pierre_core::errors::AppResult;
use serde_json::json;
use std::env;
use std::io;
use std::sync::{Arc, LazyLock, OnceLock};
use tracing::{debug, info};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::SdkTracer;

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
    if ANALYTICS_PROVIDER.set(provider).is_err() {
        debug!("analytics provider already registered; keeping the first");
    }
}

/// Process-wide [`NotifyEnricher`] for the `NotifyLayer`.
///
/// Set once at startup (before [`init_from_env`]) by the binary. The enricher
/// injects host-derived fields the call sites don't carry — a `user_email`
/// resolved from the in-process identity cache and a per-event display `emoji`
/// — into every notify event before it reaches Slack and `PostHog`. Applies to
/// whichever sinks the layer has (Slack, `PostHog`, or both), so Slack
/// identity rendering does not depend on `POSTHOG_API_KEY`.
static NOTIFY_ENRICHER: OnceLock<Arc<dyn NotifyEnricher>> = OnceLock::new();

/// Register the [`NotifyEnricher`] used by the `NotifyLayer`.
///
/// Call once before [`init_from_env`]. Subsequent calls are ignored (set-once).
pub fn set_notify_enricher(enricher: Arc<dyn NotifyEnricher>) {
    if NOTIFY_ENRICHER.set(enricher).is_err() {
        debug!("notify enricher already registered; keeping the first");
    }
}

/// Process-wide [`NotifyActions`] for the `NotifyLayer`.
///
/// Set once at startup (before [`init_from_env`]) by the binary, which owns
/// the Slack interactivity route the buttons are answered by. The layer posts
/// the elements it returns under an event's immediate Slack post — the
/// Approve and Reject buttons under `user.pending_approval`, Grant and
/// Decline under `coach_access.requested`, Raise to Enterprise under
/// `quota.limit_reached`.
static NOTIFY_ACTIONS: OnceLock<Arc<dyn NotifyActions>> = OnceLock::new();

/// Register the [`NotifyActions`] used by the `NotifyLayer`.
///
/// Call once before [`init_from_env`]. Subsequent calls are ignored (set-once).
pub fn set_notify_actions(actions: Arc<dyn NotifyActions>) {
    if NOTIFY_ACTIONS.set(actions).is_err() {
        debug!("notify actions already registered; keeping the first");
    }
}

/// Host identity for outbound alerts: the Cloud Run revision name when
/// deployed (`K_REVISION`), else the machine hostname. Resolved once per
/// process. Alert layers and the notify enricher stamp it on everything
/// they send so operators can tell a dev laptop from the deployed service
/// — both report the same `service_name`/`environment` otherwise.
static HOST_IDENTITY: LazyLock<Option<String>> = LazyLock::new(|| {
    env::var("K_REVISION")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| gethostname::gethostname().into_string().ok())
        .filter(|s| !s.is_empty())
});

/// The resolved host identity: Cloud Run revision name or machine hostname.
///
/// `None` only when `K_REVISION` is unset and the hostname is not valid
/// UTF-8 or is empty.
#[must_use]
pub fn host_identity() -> Option<&'static str> {
    HOST_IDENTITY.as_deref()
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
    /// Events one callsite may write per throttle window before the rest are
    /// dropped and counted; `0` turns the throttle off (`LOG_THROTTLE_BURST`)
    pub throttle_burst: u64,
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
                throttle_burst: DEFAULT_THROTTLE_BURST,
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
                throttle_burst: env::var("LOG_THROTTLE_BURST")
                    .ok()
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(DEFAULT_THROTTLE_BURST),
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
    fn init_telemetry(&self) -> Option<SdkTracer> {
        if !self.features.telemetry {
            return None;
        }
        match otel::build_telemetry(self) {
            Ok(Some(handle)) => {
                // `None` when OTEL_TRACES_EXPORTER=none left only metrics on.
                let tracer = handle.tracer();
                // `set` only errors if a handle is already installed (double
                // init); the first pipeline stays authoritative.
                if TELEMETRY_HANDLE.set(handle).is_err() {
                    debug!("telemetry handle already installed; keeping the first");
                }
                tracer
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
        // Every sink below masks these values: the formatter through its
        // writer, the Slack, email and PostHog layers inside tronc.
        secrets::register_env();
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
        // Returns `None` when neither SLACK_BOT_TOKEN nor POSTHOG_API_KEY is
        // set (local dev), which makes the layer a no-op via Option<Layer>
        // passthrough — the events still land in Cloud Logging through the
        // formatter layer.
        let notify_layer = Self::build_notify_layer(&self.environment);

        // Build the OTLP pipeline once (if active) and reuse its tracer across
        // the format branches. `Option<Tracer>` is subscriber-agnostic, so the
        // actual `OpenTelemetryLayer<S, _>` is constructed inside each branch
        // where `S` is known. `None` (telemetry off) makes the layer a no-op.
        #[cfg(feature = "telemetry")]
        let otel_tracer = self.init_telemetry();

        // Drops a callsite's lines past its per-window budget on the formatter
        // only: the Slack, notify and OpenTelemetry layers see every event.
        let throttle = CallsiteThrottle::new(self.features.throttle_burst, THROTTLE_WINDOW);
        match self.format {
            LogFormat::Json => {
                let json_layer = fmt::layer()
                    .with_file(self.output.location)
                    .with_line_number(self.output.location)
                    .with_thread_ids(self.output.thread)
                    .with_thread_names(self.output.thread)
                    .with_target(true)
                    .with_writer(RedactingMakeWriter::new(io::stdout))
                    .with_span_events(if self.output.spans {
                        FmtSpan::NEW | FmtSpan::CLOSE
                    } else {
                        FmtSpan::NONE
                    })
                    .json()
                    .with_filter(throttle.clone());

                let r = registry.with(json_layer).with(error_layer);
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
                    .with_writer(RedactingMakeWriter::new(io::stdout))
                    .with_span_events(if self.output.spans {
                        FmtSpan::NEW | FmtSpan::CLOSE
                    } else {
                        FmtSpan::NONE
                    })
                    .with_filter(throttle.clone());

                let r = registry.with(pretty_layer).with(error_layer);
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
                    .with_writer(RedactingMakeWriter::new(io::stdout))
                    .with_span_events(FmtSpan::NONE)
                    .with_filter(throttle.clone());

                let r = registry.with(compact_layer).with(error_layer);
                let r = r.with(notify_layer);
                #[cfg(feature = "telemetry")]
                let r = r.with(otel_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)));
                r.init();
            }
            LogFormat::Gcp => {
                let formatter = GcpFormatter::new(
                    &self.service_name,
                    &self.service_version,
                    &self.environment,
                    self.output.location,
                );
                let trace_project = env::var(request_trace::PROJECT_ENV)
                    .ok()
                    .filter(|project| !project.is_empty());
                // Link each line to the trace its request arrived with, which
                // Cloud Run's own request log line carries too.
                let formatter = match trace_project.as_deref() {
                    Some(project) => formatter.with_trace_project(project.to_owned()),
                    None => formatter,
                };
                // Link each line to its Cloud Trace span once traces export
                // and the project is known; otherwise the line is unchanged.
                #[cfg(feature = "telemetry")]
                let (formatter, dispatch_capture) =
                    match trace_project.filter(|_| otel_tracer.is_some()) {
                        Some(project) => {
                            let (link, capture) = trace_link::TraceLink::new(project);
                            (formatter.with_trace_link(link), Some(capture))
                        }
                        None => (formatter, None),
                    };
                let gcp_layer = fmt::layer()
                    .with_writer(RedactingMakeWriter::new(io::stdout))
                    .event_format(formatter)
                    .with_filter(throttle.clone());

                #[cfg(feature = "telemetry")]
                let registry = registry.with(dispatch_capture);
                let r = registry.with(gcp_layer).with(error_layer);
                let r = r.with(notify_layer);
                #[cfg(feature = "telemetry")]
                let r = r.with(otel_tracer.map(|t| tracing_opentelemetry::layer().with_tracer(t)));
                r.init();
            }
        }

        if self.features.throttle_burst > 0 {
            if let Err(e) = throttle.reporter().spawn() {
                tracing::warn!(error = %e, "log throttle reporter did not start; throttled lines go unreported");
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
        // tronc's env chain (DRAVR_NOTIFY_HOST → K_REVISION → HOSTNAME →
        // HOST) covers Cloud Run and env-providing shells; the gethostname
        // syscall fallback covers local runs where none of those are set.
        if config.host.is_none() {
            config.host = host_identity().map(str::to_owned);
        }

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
    /// The layer has two independent sinks, and either one alone builds it:
    /// Slack posts when `SLACK_BOT_TOKEN` is set, and `PostHog` captures every
    /// catalogued event (tier and consent decided by the provider) when
    /// `POSTHOG_API_KEY` is set and the binary registered an
    /// [`AnalyticsProvider`]. With neither it returns `None` — every
    /// notify-target event still lands in Cloud Logging via the formatter
    /// layer. The Slack routing rules come from the process-wide
    /// [`NOTIFY_ROUTING_PROVIDER`] which the contremaitre sync engine
    /// hot-reloads from `config/notify-routing.yaml`; they never gate a
    /// capture.
    fn build_notify_layer(environment: &str) -> Option<NotifyLayer<ContremaitreRoutingProvider>> {
        let analytics = ANALYTICS_PROVIDER
            .get()
            .cloned()
            .zip(Self::posthog_client_from_env());
        let slack = Self::notify_slack_client_from_env()
            .map(|client| (client, Arc::clone(&NOTIFY_ROUTING_PROVIDER)));
        Self::assemble_notify_layer(environment, slack, analytics)
    }

    /// The notify layer for whichever sinks are present: Slack-routed with
    /// `PostHog` beside it, Slack alone, or `PostHog` alone. `None` when
    /// neither is. The Slack sink is its client with the routing provider
    /// whose rules choose its posts. The enricher (`user_email` + emoji)
    /// feeds both sinks, so it is wired whatever the sinks are.
    fn assemble_notify_layer(
        environment: &str,
        slack: Option<(Arc<SlackClient>, Arc<ContremaitreRoutingProvider>)>,
        analytics: Option<(Arc<dyn AnalyticsProvider>, Arc<PostHogClient>)>,
    ) -> Option<NotifyLayer<ContremaitreRoutingProvider>> {
        let mut builder = match (slack, analytics) {
            (Some((slack, routing)), analytics) => {
                Self::slack_notify_builder(environment, slack, routing, analytics)
            }
            (None, Some((provider, posthog))) => {
                info!("NotifyLayer enabled without Slack: notify events reach PostHog only");
                NotifyLayer::analytics_builder(provider, posthog)
            }
            (None, None) => return None,
        };

        if let Some(enricher) = NOTIFY_ENRICHER.get().cloned() {
            builder = builder.with_enricher(enricher);
        }

        Some(builder.build())
    }

    /// The Slack-routed builder: `routing`'s rules, the buttons an event asks
    /// an operator to press under its immediate post, and the `PostHog` sink
    /// beside Slack when one is configured.
    fn slack_notify_builder(
        environment: &str,
        slack: Arc<SlackClient>,
        routing: Arc<ContremaitreRoutingProvider>,
        analytics: Option<(Arc<dyn AnalyticsProvider>, Arc<PostHogClient>)>,
    ) -> NotifyLayerBuilder<ContremaitreRoutingProvider> {
        info!(
            environment = environment,
            "NotifyLayer enabled: target=notify business events route via contremaitre"
        );
        let mut builder = NotifyLayer::builder(slack, routing, environment.to_owned());
        if let Some(actions) = NOTIFY_ACTIONS.get().cloned() {
            builder = builder.with_actions(actions);
        }
        if let Some((provider, posthog)) = analytics {
            info!("PostHog analytics sink enabled on NotifyLayer");
            builder = builder.with_analytics(provider, posthog);
        }
        builder
    }

    /// The Slack client the notify layer posts with, when `SLACK_BOT_TOKEN`
    /// is set.
    ///
    /// `SLACK_ERROR_CHANNEL` is used as a placeholder default channel
    /// for the underlying `SlackClient` — `NotifyLayer` overrides it
    /// per event using the routing rule's `channel` field, so the
    /// placeholder is only relevant when the routing provider has no
    /// rule for the emitted event (in which case the layer drops the
    /// event entirely per its `default_rule = None` configuration).
    fn notify_slack_client_from_env() -> Option<Arc<SlackClient>> {
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
        Some(Arc::new(SlackClient::new(&SlackConfig {
            bot_token,
            error_channel,
            signing_secret,
        })))
    }

    /// The `PostHog` transport, when `POSTHOG_API_KEY` is set; `POSTHOG_HOST`
    /// points it at the EU cloud or a self-hosted instance.
    fn posthog_client_from_env() -> Option<Arc<PostHogClient>> {
        let key = env::var("POSTHOG_API_KEY").ok().filter(|s| !s.is_empty())?;
        Some(Arc::new(
            match env::var("POSTHOG_HOST").ok().filter(|s| !s.is_empty()) {
                Some(host) => PostHogClient::with_host(key, host),
                None => PostHogClient::new(key),
            },
        ))
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
                throttle_burst: DEFAULT_THROTTLE_BURST,
            },
            request_id_header: "x-request-id".into(),
        }
    }
}

/// Initialize logging from environment
///
/// # Errors
///
/// Returns an error if logging initialization fails
pub fn init_from_env() -> AppResult<()> {
    LoggingConfig::from_env().init()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use dravr_tronc::notifications::{PostHogClient, SlackClient, SlackConfig};
    use dravr_tronc::notify::{AnalyticsCapture, AnalyticsProvider, NotifyLayer, NOTIFY_TARGET};
    use pierre_contremaitre::notify_routing::ContremaitreRoutingProvider;
    use tracing::{info, subscriber};
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::LoggingConfig;

    /// The event names a [`RecordingProvider`] was asked about.
    type EventLog = Arc<Mutex<Vec<String>>>;

    /// The analytics sink of a test layer, and the log of events it saw.
    fn recording_analytics() -> ((Arc<dyn AnalyticsProvider>, Arc<PostHogClient>), EventLog) {
        // Unroutable host, and the provider declines every event: nothing is
        // ever sent.
        let posthog = Arc::new(PostHogClient::with_host("phc_test", "http://127.0.0.1:1"));
        let events = EventLog::default();
        let provider: Arc<dyn AnalyticsProvider> = Arc::new(RecordingProvider {
            events: Arc::clone(&events),
        });
        ((provider, posthog), events)
    }

    /// Emit one `user.signup` notify event through `layer`.
    fn emit_signup(layer: NotifyLayer<ContremaitreRoutingProvider>) {
        subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            info!(target: NOTIFY_TARGET, event = "user.signup", "joined");
        });
    }

    /// Records the name of every event the layer asks it about, then declines
    /// it, so the `PostHog` transport is never handed a capture to send. The
    /// log is an `Arc` because the test reads it after the layer owns the
    /// provider.
    struct RecordingProvider {
        events: EventLog,
    }

    impl AnalyticsProvider for RecordingProvider {
        fn capture_for(
            &self,
            event: &str,
            _fields: &HashMap<String, String>,
        ) -> Option<AnalyticsCapture> {
            self.events
                .lock()
                .expect("event log lock") // Safe: test assertion
                .push(event.to_owned());
            None
        }
    }

    #[test]
    fn without_slack_notify_events_still_reach_posthog() {
        let (analytics, events) = recording_analytics();

        let layer = LoggingConfig::assemble_notify_layer("test", None, Some(analytics))
            .expect("analytics alone must build the notify layer"); // Safe: test assertion
        emit_signup(layer);

        assert_eq!(
            *events.lock().expect("event log lock"), // Safe: test assertion
            ["user.signup"],
            "bug: a layer without Slack must hand every notify event to the analytics provider"
        );
    }

    /// The production configuration: Slack and `PostHog` side by side.
    #[tokio::test]
    async fn with_slack_notify_events_still_reach_posthog() {
        let (analytics, events) = recording_analytics();
        let slack = Arc::new(SlackClient::new(&SlackConfig {
            bot_token: "xoxb-test".to_owned(),
            error_channel: "#test".to_owned(),
            signing_secret: None,
        }));
        // An empty routing table is disabled, so no event is ever posted:
        // the compiled-in table would route every event to a real channel.
        let routing = Arc::new(ContremaitreRoutingProvider::new_empty());

        let layer =
            LoggingConfig::assemble_notify_layer("test", Some((slack, routing)), Some(analytics))
                .expect("Slack and analytics must build the notify layer"); // Safe: test assertion
        emit_signup(layer);

        assert_eq!(
            *events.lock().expect("event log lock"), // Safe: test assertion
            ["user.signup"],
            "bug: Slack routing must not keep a notify event from the analytics provider"
        );
    }

    #[test]
    fn no_sink_builds_no_layer() {
        assert!(
            LoggingConfig::assemble_notify_layer("test", None, None).is_none(),
            "bug: with neither Slack nor PostHog there is nothing for the layer to do"
        );
    }
}
