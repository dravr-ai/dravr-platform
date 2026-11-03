// ABOUTME: Reads the Cloud Trace id an HTTP request arrived with, for log-to-request correlation
// ABOUTME: Parses W3C traceparent and X-Cloud-Trace-Context; needs no OpenTelemetry pipeline
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

/// Environment variable naming the GCP project a log line's trace lives in.
pub(crate) const PROJECT_ENV: &str = "GOOGLE_CLOUD_PROJECT";

/// Span field carrying the inbound trace id (32 lowercase hex characters).
pub(crate) const CLOUD_TRACE_ID_FIELD: &str = "cloud_trace_id";

/// The W3C trace-context header.
pub const TRACEPARENT_HEADER: &str = "traceparent";

/// Google's legacy trace header: `TRACE_ID/SPAN_ID;o=OPTIONS`.
pub const CLOUD_TRACE_CONTEXT_HEADER: &str = "x-cloud-trace-context";

/// The trace id a request arrived with: `traceparent` first, then
/// `X-Cloud-Trace-Context`. `None` when neither carries a well-formed id.
#[must_use]
pub fn inbound_trace_id(
    traceparent: Option<&str>,
    cloud_trace_context: Option<&str>,
) -> Option<String> {
    traceparent
        .and_then(from_traceparent)
        .or_else(|| cloud_trace_context.and_then(from_cloud_trace_context))
}

/// `projects/<project>/traces/<trace id>`, the form Cloud Logging links on.
#[must_use]
pub(crate) fn trace_resource(project: &str, trace_id: &str) -> String {
    format!("projects/{project}/traces/{trace_id}")
}

/// The trace id of a `version-traceid-parentid-flags` header.
fn from_traceparent(header: &str) -> Option<String> {
    let mut parts = header.trim().split('-');
    let version = parts.next()?;
    let trace = parts.next()?;
    let parent = parts.next()?;
    let flags = parts.next()?;
    let well_formed = version.len() == 2
        && is_hex(version)
        && !version.eq_ignore_ascii_case("ff")
        && parent.len() == 16
        && is_hex(parent)
        && flags.len() == 2
        && is_hex(flags);
    if well_formed {
        trace_id(trace)
    } else {
        None
    }
}

/// The trace id of a `TRACE_ID/SPAN_ID;o=OPTIONS` header.
fn from_cloud_trace_context(header: &str) -> Option<String> {
    header.trim().split('/').next().and_then(trace_id)
}

/// `raw` lowercased when it is 32 hex characters and not all zero (the W3C
/// spec's invalid id).
fn trace_id(raw: &str) -> Option<String> {
    let valid = raw.len() == 32 && is_hex(raw) && raw.bytes().any(|b| b != b'0');
    valid.then(|| raw.to_ascii_lowercase())
}

fn is_hex(value: &str) -> bool {
    value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

    #[test]
    fn reads_the_trace_id_of_a_traceparent() {
        let header = format!("00-{TRACE}-00f067aa0ba902b7-01");
        assert_eq!(
            inbound_trace_id(Some(&header), None).as_deref(),
            Some(TRACE)
        );
    }

    #[test]
    fn lowercases_an_uppercase_trace_id() {
        let header = format!("00-{}-00f067aa0ba902b7-00", TRACE.to_ascii_uppercase());
        assert_eq!(
            inbound_trace_id(Some(&header), None).as_deref(),
            Some(TRACE)
        );
    }

    #[test]
    fn falls_back_to_x_cloud_trace_context() {
        let header = format!("{TRACE}/1234567890;o=1");
        assert_eq!(
            inbound_trace_id(None, Some(&header)).as_deref(),
            Some(TRACE)
        );
        assert_eq!(
            inbound_trace_id(Some("garbage"), Some(TRACE)).as_deref(),
            Some(TRACE)
        );
    }

    #[test]
    fn prefers_traceparent_over_x_cloud_trace_context() {
        let other = "0af7651916cd43dd8448eb211c80319c";
        let header = format!("00-{TRACE}-00f067aa0ba902b7-01");
        assert_eq!(
            inbound_trace_id(Some(&header), Some(other)).as_deref(),
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
            assert_eq!(inbound_trace_id(Some(&header), None), None, "{header}");
        }
        assert_eq!(inbound_trace_id(None, Some("not-a-trace/1")), None);
        assert_eq!(inbound_trace_id(None, None), None);
    }

    #[test]
    fn names_the_trace_resource_cloud_logging_links_on() {
        assert_eq!(
            trace_resource("dravr-dev", TRACE),
            format!("projects/dravr-dev/traces/{TRACE}")
        );
    }
}
