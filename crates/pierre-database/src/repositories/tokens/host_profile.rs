// ABOUTME: The Host Profile a public client's metadata document declares (owl §6.2 carrier A), stored on its row
// ABOUTME: Written where the document is read, read when a step-up child grant records its basis (carnet#1544)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Record the Host Profile a client's metadata document declares
/// (`mcp_host_profile`, owl §6.2 carrier A), or none. Only the metadata
/// document resolver writes it, so a client registered by DCR has none.
pub(crate) const SET_OAUTH2_CLIENT_HOST_PROFILE_SQL: &str = r"
            UPDATE oauth2_clients SET host_profile = $2
            WHERE client_id = $1
            ";

/// The Host Profile a client's metadata document declared.
pub(crate) const GET_OAUTH2_CLIENT_HOST_PROFILE_SQL: &str = r"
            SELECT host_profile FROM oauth2_clients WHERE client_id = $1
            ";
