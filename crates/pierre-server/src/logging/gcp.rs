// ABOUTME: Google Cloud Logging structured-JSON formatter for Cloud Run.
// ABOUTME: Maps tracing events to Cloud Logging's LogEntry schema via stdout JSON.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Structured JSON formatter matching Google Cloud Logging's special-fields spec
//! (<https://cloud.google.com/logging/docs/structured-logging>). Emits one JSON
//! object per tracing event to stdout; Cloud Run's log agent promotes the
//! `severity`, `message`, `time`, and `logging.googleapis.com/*` fields onto
//! the `LogEntry` so alert policies keyed on severity can fire.

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;

use chrono::Utc;
use serde_json::{json, Map, Value};
use tracing::{field::Field, field::Visit, Event, Level, Subscriber};
use tracing_subscriber::fmt::{format::Writer, FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Key used by tracing for the primary message field.
const MESSAGE_FIELD: &str = "message";

/// Cloud Logging special field names (exact strings required by the spec).
const SEVERITY_KEY: &str = "severity";
const TIME_KEY: &str = "time";
const SOURCE_LOCATION_KEY: &str = "logging.googleapis.com/sourceLocation";
const LABELS_KEY: &str = "logging.googleapis.com/labels";
const SPAN_ID_KEY: &str = "logging.googleapis.com/spanId";

/// Formatter emitting Cloud Logging-compatible JSON lines.
pub struct GcpFormatter {
    /// Pre-baked labels included on every emitted event.
    labels: BTreeMap<String, String>,
    /// Include `sourceLocation` for each event when true.
    include_source_location: bool,
}

impl GcpFormatter {
    /// Build a formatter with the given service labels. Labels are emitted on
    /// every event under `logging.googleapis.com/labels`.
    pub fn new(
        service_name: &str,
        service_version: &str,
        environment: &str,
        include_source_location: bool,
    ) -> Self {
        let mut labels = BTreeMap::new();
        labels.insert("service.name".to_owned(), service_name.to_owned());
        labels.insert("service.version".to_owned(), service_version.to_owned());
        labels.insert("environment".to_owned(), environment.to_owned());
        Self {
            labels,
            include_source_location,
        }
    }
}

impl<S, N> FormatEvent<S, N> for GcpFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();

        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);

        let mut entry = Map::new();
        entry.insert(
            SEVERITY_KEY.to_owned(),
            Value::String(severity_for(*metadata.level()).to_owned()),
        );

        let message = visitor.message.unwrap_or_default();
        entry.insert(MESSAGE_FIELD.to_owned(), Value::String(message));

        entry.insert(
            TIME_KEY.to_owned(),
            Value::String(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)),
        );

        let mut labels = Map::new();
        for (k, v) in &self.labels {
            labels.insert(k.clone(), Value::String(v.clone()));
        }
        labels.insert(
            "rust.target".to_owned(),
            Value::String(metadata.target().to_owned()),
        );
        entry.insert(LABELS_KEY.to_owned(), Value::Object(labels));

        if self.include_source_location {
            if let (Some(file), Some(line)) = (metadata.file(), metadata.line()) {
                entry.insert(
                    SOURCE_LOCATION_KEY.to_owned(),
                    json!({
                        "file": file,
                        "line": line.to_string(),
                        "function": metadata.module_path().unwrap_or(""),
                    }),
                );
            }
        }

        if let Some(span) = ctx.lookup_current() {
            entry.insert(
                SPAN_ID_KEY.to_owned(),
                Value::String(format!("{:016x}", span.id().into_u64())),
            );
        }

        for (k, v) in visitor.fields {
            // Avoid clobbering canonical keys if a user event uses a reserved name.
            if !entry.contains_key(&k) {
                entry.insert(k, v);
            }
        }

        let serialized = serde_json::to_string(&Value::Object(entry)).map_err(|_| fmt::Error)?;
        writeln!(writer, "{serialized}")
    }
}

/// Map a `tracing::Level` to a Cloud Logging severity string.
fn severity_for(level: Level) -> &'static str {
    // Cloud Logging accepts DEBUG, INFO, NOTICE, WARNING, ERROR, CRITICAL, ALERT, EMERGENCY.
    match level {
        Level::TRACE | Level::DEBUG => "DEBUG",
        Level::INFO => "INFO",
        Level::WARN => "WARNING",
        Level::ERROR => "ERROR",
    }
}

/// Visitor collecting event fields into a serde_json-friendly map.
///
/// The `message` field is kept separate so it can be promoted to the
/// top-level `message` key in the emitted JSON.
#[derive(Default)]
struct JsonVisitor {
    message: Option<String>,
    fields: Map<String, Value>,
}

impl Visit for JsonVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        self.store(field.name(), Value::String(rendered));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.store(field.name(), Value::String(value.to_owned()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.store(field.name(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.store(field.name(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.store(field.name(), Value::Number(value.into()));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        match serde_json::Number::from_f64(value) {
            Some(n) => self.store(field.name(), Value::Number(n)),
            None => self.store(field.name(), Value::String(value.to_string())),
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn StdError + 'static)) {
        self.store(field.name(), Value::String(value.to_string()));
    }
}

impl JsonVisitor {
    fn store(&mut self, name: &str, value: Value) {
        if name == MESSAGE_FIELD {
            if let Value::String(s) = value {
                self.message = Some(s);
                return;
            }
        }
        self.fields.insert(name.to_owned(), value);
    }
}
