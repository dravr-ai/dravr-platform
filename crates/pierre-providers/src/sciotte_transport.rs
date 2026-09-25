// ABOUTME: How a request to the dravr-sciotte scraper failed when no HTTP response came back.
// ABOUTME: Tells a timeout, an unreachable service and a connection closed mid-request apart, naming the request id.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Transport failures of the remote sciotte client
//! ([`RemoteSciotteClient`](crate::sciotte_remote::RemoteSciotteClient)).
//!
//! A request that produced no response is described by what the transport
//! saw — the client's timeout, no connection, or a connection that closed
//! after the request went out — because that is what tells a slow scrape from
//! a service that dropped the request. The client re-sends an idempotent read
//! on [`TransportFailure::ClosedBeforeResponse`] and nothing else.

use std::error::Error as StdError;
use std::io;
use std::time::Instant;

use dravr_tronc::server::request_guard::REQUEST_ID_HEADER;
use pierre_core::errors::{AppError, ErrorCode};

/// How a request to the scraper failed when no HTTP response came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailure {
    /// The client's own timeout elapsed first.
    TimedOut,
    /// No connection to the service could be opened.
    Unreachable,
    /// The connection closed after the request went out and before any
    /// response: the peer went away mid-request.
    ClosedBeforeResponse,
    /// The request could not be built or sent for another reason.
    Other,
}

impl TransportFailure {
    /// Classify the error a send failed with.
    pub fn of(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::TimedOut
        } else if error.is_connect() {
            Self::Unreachable
        } else if closed_before_response(error) {
            Self::ClosedBeforeResponse
        } else {
            Self::Other
        }
    }

    /// What happened, as the error message says it.
    const fn describe(self) -> &'static str {
        match self {
            Self::TimedOut => "timed out waiting for a response",
            Self::Unreachable => "could not connect to the service",
            Self::ClosedBeforeResponse => "saw the connection close before any response",
            Self::Other => "could not be sent",
        }
    }
}

/// Whether a send failed because the connection closed before any response.
///
/// hyper reports that as an incomplete message (the peer closed mid-exchange)
/// or a canceled request (the pooled connection closed before it could be
/// written); a reset or broken connection surfaces as the I/O error beneath.
fn closed_before_response(error: &reqwest::Error) -> bool {
    let mut cause: Option<&(dyn StdError + 'static)> = error.source();
    while let Some(current) = cause {
        if let Some(hyper_error) = current.downcast_ref::<hyper::Error>() {
            if hyper_error.is_incomplete_message() || hyper_error.is_canceled() {
                return true;
            }
        }
        if let Some(io_error) = current.downcast_ref::<io::Error>() {
            if matches!(
                io_error.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::UnexpectedEof
            ) {
                return true;
            }
        }
        cause = current.source();
    }
    false
}

/// `error` and every cause beneath it, `: `-joined — without the URL reqwest
/// prints, whose query string carries athlete ids.
fn error_chain(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut chain = error.to_string();
    let mut cause = error.source();
    while let Some(current) = cause {
        chain.push_str(": ");
        chain.push_str(&current.to_string());
        cause = current.source();
    }
    chain
}

/// The error for a request that produced no response.
///
/// A timeout, an unreachable service and a connection closed mid-request are
/// the provider being unavailable at the moment, so they are
/// [`ErrorCode::ExternalServiceUnavailable`]; a request that could not be sent
/// at all stays an internal fault. The message names the request id — the one
/// the service logged it under, when it got that far — how long it ran, and
/// the transport's own cause chain, which is what tells a client-side timeout
/// from a service that dropped the request.
pub fn transport_error(
    operation: &str,
    request_id: &str,
    started: Instant,
    failure: TransportFailure,
    error: reqwest::Error,
) -> AppError {
    let message = format!(
        "sciotte {operation} request ({REQUEST_ID_HEADER} {request_id}) {} after {} ms: {}",
        failure.describe(),
        started.elapsed().as_millis(),
        error_chain(error)
    );
    match failure {
        TransportFailure::Other => AppError::internal(message),
        TransportFailure::TimedOut
        | TransportFailure::Unreachable
        | TransportFailure::ClosedBeforeResponse => {
            AppError::new(ErrorCode::ExternalServiceUnavailable, message)
        }
    }
}
