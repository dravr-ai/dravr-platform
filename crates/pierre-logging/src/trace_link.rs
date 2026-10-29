// ABOUTME: Links Cloud Logging lines to their Cloud Trace span (logging.googleapis.com/trace + spanId)
// ABOUTME: Compiled with `telemetry`; inert until the OTel layer is installed and GOOGLE_CLOUD_PROJECT is set
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Log-to-trace correlation for the GCP formatter.
//!
//! Cloud Logging joins a log line to its trace through
//! `logging.googleapis.com/trace` (`projects/<project>/traces/<trace id>`) and
//! `logging.googleapis.com/spanId` (the OpenTelemetry span id). Those ids live
//! in the `tracing-opentelemetry` layer's span data, which 0.33 exposes only
//! through [`tracing_opentelemetry::get_otel_context`] — and that needs the
//! [`Dispatch`]. A formatter has no handle on it, and a nested
//! `dispatcher::get_default` inside event dispatch answers with the no-op
//! subscriber, so [`DispatchCapture`] keeps a weak handle from
//! `on_register_dispatch` and [`TraceLink`] reads the ids through it.
//!
//! Reading the context builds the OTel span if it was not built yet, so a
//! remote parent (`OpenTelemetrySpanExt::set_parent`) must be set before the
//! span's first log line — the same rule as before its first child span.

use std::sync::{Arc, OnceLock};

use opentelemetry::trace::TraceContextExt;
use tracing::dispatcher::WeakDispatch;
use tracing::span::Id;
use tracing::{Dispatch, Subscriber};
use tracing_opentelemetry::get_otel_context;
use tracing_subscriber::layer::Layer;

/// Environment variable naming the GCP project traces are written to.
pub(crate) const PROJECT_ENV: &str = "GOOGLE_CLOUD_PROJECT";

/// The trace fields one log line carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraceFields {
    /// `projects/<project>/traces/<32-hex trace id>`.
    pub(crate) trace: String,
    /// 16-hex OpenTelemetry span id.
    pub(crate) span_id: String,
    /// Whether the sampler kept this trace.
    pub(crate) sampled: bool,
}

/// Resolves the Cloud Trace ids of a span for the formatter.
#[derive(Debug, Clone)]
pub(crate) struct TraceLink {
    project: String,
    dispatch: Arc<OnceLock<WeakDispatch>>,
}

impl TraceLink {
    /// A link writing traces under `project`, and the layer that hands it the
    /// dispatch. Install the layer on the same subscriber as the formatter.
    #[must_use]
    pub(crate) fn new(project: String) -> (Self, DispatchCapture) {
        let dispatch = Arc::new(OnceLock::new());
        (
            Self {
                project,
                dispatch: Arc::clone(&dispatch),
            },
            DispatchCapture { dispatch },
        )
    }

    /// The trace fields of `span`, or `None` when no OpenTelemetry layer is
    /// installed or the span has no valid context. The caller must not hold
    /// the span's extensions lock.
    #[must_use]
    pub(crate) fn fields(&self, span: &Id) -> Option<TraceFields> {
        let dispatch = self.dispatch.get()?.upgrade()?;
        let context = get_otel_context(span, &dispatch)?;
        let otel_span = context.span();
        let span_context = otel_span.span_context();
        if !span_context.is_valid() {
            return None;
        }
        Some(TraceFields {
            trace: format!(
                "projects/{}/traces/{}",
                self.project,
                span_context.trace_id()
            ),
            span_id: span_context.span_id().to_string(),
            sampled: span_context.is_sampled(),
        })
    }
}

/// Layer that hands the subscriber's [`Dispatch`] to a [`TraceLink`].
#[derive(Debug)]
pub(crate) struct DispatchCapture {
    dispatch: Arc<OnceLock<WeakDispatch>>,
}

impl<S: Subscriber> Layer<S> for DispatchCapture {
    fn on_register_dispatch(&self, subscriber: &Dispatch) {
        // First registration wins: a layer is registered with one subscriber,
        // so a second call carries the same dispatch and is dropped.
        self.dispatch.get_or_init(|| subscriber.downgrade());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Mutex, PoisonError};

    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use serde_json::Value;
    use tracing::subscriber::with_default;
    use tracing::{info, info_span};
    use tracing_subscriber::fmt::{self, MakeWriter};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::registry;

    use crate::gcp::GcpFormatter;
    use crate::span_fields::SpanFieldStorage;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn last_line(buffer: &Buffer) -> Value {
        let bytes = buffer
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let text = String::from_utf8(bytes).unwrap_or_default();
        let line = text.lines().last().unwrap_or_default().to_owned();
        serde_json::from_str(&line).unwrap_or(Value::Null)
    }

    fn formatter(link: Option<TraceLink>) -> GcpFormatter {
        let formatter = GcpFormatter::new("svc", "0", "test", false);
        match link {
            Some(link) => formatter.with_trace_link(link),
            None => formatter,
        }
    }

    #[test]
    fn a_log_line_inside_a_traced_span_names_its_trace() {
        let buffer = Buffer::default();
        let provider = SdkTracerProvider::builder().build();
        let (link, capture) = TraceLink::new("dravr-dev".to_owned());
        let subscriber = registry()
            .with(SpanFieldStorage)
            .with(capture)
            .with(
                fmt::layer()
                    .with_writer(buffer.clone())
                    .event_format(formatter(Some(link))),
            )
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));

        with_default(subscriber, || {
            let request = info_span!("request");
            let _entered = request.enter();
            info!("inside");
        });

        let logged = last_line(&buffer);
        let trace = logged["logging.googleapis.com/trace"]
            .as_str()
            .unwrap_or_default();
        let trace_id = trace
            .strip_prefix("projects/dravr-dev/traces/")
            .unwrap_or_default();
        assert_eq!(trace_id.len(), 32, "trace field was {trace:?}");
        assert!(
            trace_id.chars().all(|c| c.is_ascii_hexdigit()),
            "bug: the trace id must be hex, was {trace_id:?}"
        );
        assert_ne!(
            trace_id,
            "0".repeat(32),
            "bug: an invalid (all-zero) trace id must never be written"
        );
        let span_id = logged["logging.googleapis.com/spanId"]
            .as_str()
            .unwrap_or_default();
        assert_eq!(span_id.len(), 16, "spanId was {span_id:?}");
        assert_eq!(
            logged["logging.googleapis.com/trace_sampled"],
            Value::Bool(true),
            "bug: the default sampler keeps the trace, so the line must say so"
        );
    }

    #[test]
    fn without_an_otel_layer_no_trace_field_is_written() {
        let buffer = Buffer::default();
        let (link, capture) = TraceLink::new("dravr-dev".to_owned());
        let subscriber = registry().with(SpanFieldStorage).with(capture).with(
            fmt::layer()
                .with_writer(buffer.clone())
                .event_format(formatter(Some(link))),
        );

        with_default(subscriber, || {
            let request = info_span!("request");
            let _entered = request.enter();
            info!("inside");
        });

        let logged = last_line(&buffer);
        assert!(
            logged.get("logging.googleapis.com/trace").is_none(),
            "bug: without an OTel layer no trace can be named"
        );
        assert!(
            logged.get("logging.googleapis.com/spanId").is_some(),
            "bug: the local span id must still be written without an OTel layer"
        );
    }
}
