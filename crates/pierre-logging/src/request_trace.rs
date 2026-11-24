// ABOUTME: Reads the Cloud Trace id an HTTP request arrived with, for log-to-request correlation
// ABOUTME: Takes the W3C trace context dravr-tronc validated, else parses X-Cloud-Trace-Context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The trace id a request arrived with.
//!
//! Cloud Run stamps every request it routes with a trace id, carried in the
//! `traceparent` and `X-Cloud-Trace-Context` headers, and writes the same id on
//! its own request log line. Recording that id on the request span lets the
//! GCP formatter put it on every app line the request logs
//! (`logging.googleapis.com/trace`), so Logs Explorer groups them with the
//! request — without exporting a single span.
//!
//! The `traceparent` header is read and validated by dravr-tronc's
//! [`TraceContext`], the one W3C parser every dravr service shares; only
//! Google's legacy header is parsed here.

use dravr_tronc::server::trace_context::TraceContext;

/// Environment variable naming the GCP project a log line's trace lives in.
pub(crate) const PROJECT_ENV: &str = "GOOGLE_CLOUD_PROJECT";

/// Span field carrying the inbound trace id (32 lowercase hex characters).
pub(crate) const CLOUD_TRACE_ID_FIELD: &str = "cloud_trace_id";

/// Google's legacy trace header: `TRACE_ID/SPAN_ID;o=OPTIONS`.
pub const CLOUD_TRACE_CONTEXT_HEADER: &str = "x-cloud-trace-context";

/// The trace id a request arrived with: the validated W3C trace context
/// first, then `X-Cloud-Trace-Context`. `None` when neither carries a
/// well-formed id.
#[must_use]
pub fn inbound_trace_id(
    trace_context: Option<&TraceContext>,
    cloud_trace_context: Option<&str>,
) -> Option<String> {
    trace_context
        .map(|context| context.trace_id().to_owned())
        .or_else(|| cloud_trace_context.and_then(from_cloud_trace_context))
}

/// `projects/<project>/traces/<trace id>`, the form Cloud Logging links on.
#[must_use]
pub(crate) fn trace_resource(project: &str, trace_id: &str) -> String {
    format!("projects/{project}/traces/{trace_id}")
}

/// The trace id of a `TRACE_ID/SPAN_ID;o=OPTIONS` header: its first segment,
/// lowercased, when that is 32 hex characters and not all zero.
fn from_cloud_trace_context(header: &str) -> Option<String> {
    let raw = header.trim().split('/').next()?;
    let valid = raw.len() == 32
        && raw.bytes().all(|b| b.is_ascii_hexdigit())
        && raw.bytes().any(|b| b != b'0');
    valid.then(|| raw.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const OTHER: &str = "0af7651916cd43dd8448eb211c80319c";

    /// What the request span records for these two header values, the
    /// `traceparent` going through dravr-tronc's parser as it does in the
    /// server.
    fn trace_id_of(traceparent: Option<&str>, cloud_trace_context: Option<&str>) -> Option<String> {
        let context = traceparent.and_then(|header| TraceContext::parse(header, None));
        inbound_trace_id(context.as_ref(), cloud_trace_context)
    }

    #[test]
    fn reads_the_trace_id_of_a_traceparent() {
        let header = format!("00-{TRACE}-00f067aa0ba902b7-01");
        assert_eq!(trace_id_of(Some(&header), None).as_deref(), Some(TRACE));
        let padded = format!("  00-{TRACE}-00f067aa0ba902b7-00 ");
        assert_eq!(trace_id_of(Some(&padded), None).as_deref(), Some(TRACE));
    }

    #[test]
    fn falls_back_to_x_cloud_trace_context() {
        let header = format!("{TRACE}/1234567890;o=1");
        assert_eq!(trace_id_of(None, Some(&header)).as_deref(), Some(TRACE));
        assert_eq!(
            trace_id_of(Some("garbage"), Some(TRACE)).as_deref(),
            Some(TRACE)
        );
    }

    #[test]
    fn lowercases_an_uppercase_x_cloud_trace_context() {
        let header = format!("{}/1;o=1", TRACE.to_ascii_uppercase());
        assert_eq!(trace_id_of(None, Some(&header)).as_deref(), Some(TRACE));
    }

    #[test]
    fn prefers_traceparent_over_x_cloud_trace_context() {
        let header = format!("00-{TRACE}-00f067aa0ba902b7-01");
        assert_eq!(
            trace_id_of(Some(&header), Some(OTHER)).as_deref(),
            Some(TRACE)
        );
    }

    #[test]
    fn refuses_malformed_ids() {
        let zero = "0".repeat(32);
        for header in [
            format!("00-{zero}-00f067aa0ba902b7-01"),
            format!("ff-{TRACE}-00f067aa0ba902b7-01"),
            format!("00-{TRACE}-short-01"),
            format!("00-{}-00f067aa0ba902b7-01", &TRACE[..31]),
            format!("00-{TRACE}"),
            "00-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-00f067aa0ba902b7-01".to_owned(),
        ] {
            assert_eq!(trace_id_of(Some(&header), None), None, "{header}");
        }
        assert_eq!(trace_id_of(None, Some("not-a-trace/1")), None);
        assert_eq!(trace_id_of(None, Some(&format!("{zero}/1"))), None);
        assert_eq!(trace_id_of(None, None), None);
    }

    /// The shared W3C parser is stricter than the hand-written one it replaced, which lowercased an
    /// uppercase id, accepted an all-zero parent id and ignored anything after the flags.
    ///
    /// Each of these now carries no trace of its own, so the request falls back to `X-Cloud-Trace-
    /// Context`.
    #[test]
    fn a_traceparent_the_w3c_parser_refuses_falls_back_to_x_cloud_trace_context() {
        for header in [
            format!("00-{}-00f067aa0ba902b7-01", TRACE.to_ascii_uppercase()),
            format!("00-{TRACE}-00F067AA0BA902B7-01"),
            format!("00-{TRACE}-0000000000000000-01"),
            format!("00-{TRACE}-00f067aa0ba902b7-01-extra"),
        ] {
            assert_eq!(trace_id_of(Some(&header), None), None, "{header}");
            assert_eq!(
                trace_id_of(Some(&header), Some(OTHER)).as_deref(),
                Some(OTHER),
                "{header}"
            );
        }
    }

    #[test]
    fn names_the_trace_resource_cloud_logging_links_on() {
        assert_eq!(
            trace_resource("dravr-dev", TRACE),
            format!("projects/dravr-dev/traces/{TRACE}")
        );
    }
}
