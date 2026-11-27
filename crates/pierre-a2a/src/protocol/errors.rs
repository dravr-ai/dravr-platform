// ABOUTME: The JSON-RPC and A2A-spec error responses the A2A server answers with
// ABOUTME: Plain JSON-RPC codes, spec codes with their ErrorInfo detail, and the 401-mapped auth refusals

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::error::{INVALID_PARAMS, SERVER_ERROR_START};
use serde_json::Value;

use super::A2AServer;
use crate::protocol_types::{authentication_required_info, error_info, A2ASpecError};
use crate::{A2AResponse, JSONRPC_VERSION};

impl A2AServer {
    /// Create a standard A2A JSON-RPC error response
    pub(super) fn a2a_error(
        code: i32,
        message: impl Into<String>,
        request_id: Option<Value>,
    ) -> A2AResponse {
        A2AResponse::error(request_id, code, message)
    }

    /// Create an A2A-spec error response (`-32001..-32009`) with the
    /// mandatory `google.rpc.ErrorInfo` detail.
    pub(super) fn spec_error(err: A2ASpecError, request_id: Option<Value>) -> A2AResponse {
        A2AResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            result: None,
            error: Some(err.into()),
            id: request_id,
        }
    }

    /// Create an A2A-spec error response carrying a caller-facing message in
    /// place of the variant's generic text, keeping the spec code and the
    /// mandatory `google.rpc.ErrorInfo` detail.
    pub(super) fn spec_error_with_message(
        err: A2ASpecError,
        message: impl Into<String>,
        request_id: Option<Value>,
    ) -> A2AResponse {
        A2AResponse::error_with_data(request_id, err.code(), message, error_info(err.reason()))
    }

    /// Create an invalid-params (`-32602`) error response.
    pub(super) fn invalid_params(
        message: impl Into<String>,
        request_id: Option<Value>,
    ) -> A2AResponse {
        Self::a2a_error(INVALID_PARAMS, message, request_id)
    }

    /// Create an authentication-failure response. Carries the
    /// `AUTHENTICATION_REQUIRED` reason so HTTP bindings can map it to 401.
    pub(super) fn auth_error(message: impl Into<String>, request_id: Option<Value>) -> A2AResponse {
        A2AResponse::error_with_data(
            request_id,
            SERVER_ERROR_START,
            message,
            authentication_required_info(None),
        )
    }

    /// An authentication refusal by the `DPoP` binding rules, naming the
    /// RFC 9449 error `code` the 401 challenge carries.
    pub(super) fn dpop_auth_error(
        message: impl Into<String>,
        code: &str,
        request_id: Option<Value>,
    ) -> A2AResponse {
        A2AResponse::error_with_data(
            request_id,
            SERVER_ERROR_START,
            message,
            authentication_required_info(Some(code)),
        )
    }

    /// Create a standard server-not-configured error response
    pub(super) fn server_not_configured_error(request_id: Option<Value>) -> A2AResponse {
        Self::a2a_error(
            SERVER_ERROR_START,
            "A2A server not properly configured",
            request_id,
        )
    }
}
