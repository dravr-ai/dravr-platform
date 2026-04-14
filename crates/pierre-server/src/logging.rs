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

/// Re-export tenant logging utilities
pub use tenant::{
    record_performance_metrics, record_request_context, record_tenant_context, ProviderApiContext,
    TenantLogger,
};

use gcp::GcpFormatter;

use crate::constants::service_names;
use crate::errors::AppResult;
use dravr_tronc::notifications::{
    EmailClient, ErrorNotificationLayer, NotificationConfig, SlackClient,
};
use serde_json::json;
use std::env;
use std::io;
use tracing::info;
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

// OpenTelemetry support disabled temporarily due to version compatibility issues

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
                telemetry: false,
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
                telemetry: is_production || env::var("ENABLE_TELEMETRY").is_ok(),
                truncate_mcp: env::var("MCP_LOG_TRUNCATE")
                    .map(|v| v != "false" && v != "0")
                    .unwrap_or(true), // Default to true (truncated) unless explicitly disabled
            },
            request_id_header: env::var("REQUEST_ID_HEADER")
                .unwrap_or_else(|_| "x-request-id".into()),
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

        // Create base registry
        let registry = tracing_subscriber::registry().with(env_filter);

        // Build optional error notification layer (active when SLACK_ERROR_CHANNEL is set)
        let error_layer =
            Self::build_error_notification_layer(&self.service_name, &self.environment);

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
                    .json();

                registry.with(json_layer).with(error_layer).init();
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
                    });

                registry.with(pretty_layer).with(error_layer).init();
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
                    .with_span_events(FmtSpan::NONE);

                registry.with(compact_layer).with(error_layer).init();
            }
            LogFormat::Gcp => {
                let gcp_layer =
                    fmt::layer()
                        .with_writer(io::stdout)
                        .event_format(GcpFormatter::new(
                            &self.service_name,
                            &self.service_version,
                            &self.environment,
                            self.output.location,
                        ));

                registry.with(gcp_layer).with(error_layer).init();
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

    /// Create `OpenTelemetry` layer for distributed tracing
    ///
    /// Currently disabled due to dependency version conflicts with tokio-tungstenite.
    /// `OpenTelemetry` requires specific versions that conflict with `WebSocket` dependencies.
    #[allow(dead_code, clippy::unused_self, clippy::unnecessary_wraps)]
    fn create_telemetry_layer(&self) -> AppResult<()> {
        // OpenTelemetry integration disabled due to version compatibility issues
        // Can be enabled once dependency conflicts are resolved
        info!("`OpenTelemetry` layer creation requested but disabled due to dependency conflicts");
        Ok(())
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
                telemetry: true,
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
