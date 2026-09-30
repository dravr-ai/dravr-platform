// ABOUTME: Tool-specific error types for the pluggable tools architecture
// ABOUTME: Provides structured errors that integrate with the main AppError system
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Tool Error Types
//!
//! Provides structured error types for tool-related operations:
//! - `ToolError` - Errors specific to tool execution and registry
//! - Conversion traits to `AppError` for HTTP response formatting

use std::error::Error;
use std::fmt;

/// Errors specific to tool operations.
///
/// These errors provide detailed context for tool-related failures
/// while maintaining compatibility with the main `AppError` system.
#[derive(Debug, Clone)]
pub enum ToolError {
    /// Tool was not found in the registry
    NotFound {
        /// Name of the requested tool
        tool_name: String,
    },
    /// Tool is disabled for this tenant
    DisabledForTenant {
        /// Name of the disabled tool
        tool_name: String,
        /// Tenant ID where the tool is disabled
        tenant_id: uuid::Uuid,
    },
    /// Tool requires admin privileges
    AdminRequired {
        /// Name of the tool requiring admin access
        tool_name: String,
    },
    /// Tool requires a connected provider
    ProviderRequired {
        /// Name of the tool requiring a provider
        tool_name: String,
        /// Optional: specific provider needed
        provider: Option<String>,
    },
    /// Tool parameter validation failed
    InvalidParameter {
        /// Name of the tool
        tool_name: String,
        /// Name of the invalid parameter
        parameter: String,
        /// Reason the parameter is invalid
        reason: String,
    },
    /// Required parameter is missing
    MissingParameter {
        /// Name of the tool
        tool_name: String,
        /// Name of the missing parameter
        parameter: String,
    },
    /// Tool execution failed
    ExecutionFailed {
        /// Name of the tool that failed
        tool_name: String,
        /// Details about the failure
        details: String,
    },
    /// Tool is already registered (for registry operations)
    AlreadyRegistered {
        /// Name of the already-registered tool
        tool_name: String,
    },
    /// Tool capability check failed
    CapabilityMismatch {
        /// Name of the tool
        tool_name: String,
        /// Required capability that was missing
        required: String,
    },
}

impl ToolError {
    /// Create a "not found" error
    #[must_use]
    pub fn not_found(tool_name: impl Into<String>) -> Self {
        Self::NotFound {
            tool_name: tool_name.into(),
        }
    }

    /// Create an "admin required" error
    #[must_use]
    pub fn admin_required(tool_name: impl Into<String>) -> Self {
        Self::AdminRequired {
            tool_name: tool_name.into(),
        }
    }

    /// Create a "missing parameter" error
    #[must_use]
    pub fn missing_parameter(tool_name: impl Into<String>, parameter: impl Into<String>) -> Self {
        Self::MissingParameter {
            tool_name: tool_name.into(),
            parameter: parameter.into(),
        }
    }

    /// Get the tool name associated with this error
    #[must_use]
    pub fn tool_name(&self) -> &str {
        match self {
            Self::NotFound { tool_name }
            | Self::DisabledForTenant { tool_name, .. }
            | Self::AdminRequired { tool_name }
            | Self::ProviderRequired { tool_name, .. }
            | Self::InvalidParameter { tool_name, .. }
            | Self::MissingParameter { tool_name, .. }
            | Self::ExecutionFailed { tool_name, .. }
            | Self::AlreadyRegistered { tool_name }
            | Self::CapabilityMismatch { tool_name, .. } => tool_name,
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { tool_name } => {
                write!(f, "Tool '{tool_name}' not found")
            }
            Self::DisabledForTenant {
                tool_name,
                tenant_id,
            } => {
                write!(f, "Tool '{tool_name}' is disabled for tenant {tenant_id}")
            }
            Self::AdminRequired { tool_name } => {
                write!(f, "Tool '{tool_name}' requires admin privileges")
            }
            Self::ProviderRequired {
                tool_name,
                provider: Some(p),
            } => {
                write!(
                    f,
                    "Tool '{tool_name}' requires provider '{p}' to be connected"
                )
            }
            Self::ProviderRequired {
                tool_name,
                provider: None,
            } => {
                write!(
                    f,
                    "Tool '{tool_name}' requires a connected fitness provider"
                )
            }
            Self::InvalidParameter {
                tool_name,
                parameter,
                reason,
            } => {
                write!(
                    f,
                    "Invalid parameter '{parameter}' for tool '{tool_name}': {reason}"
                )
            }
            Self::MissingParameter {
                tool_name,
                parameter,
            } => {
                write!(
                    f,
                    "Missing required parameter '{parameter}' for tool '{tool_name}'"
                )
            }
            Self::ExecutionFailed { tool_name, details } => {
                write!(f, "Tool '{tool_name}' execution failed: {details}")
            }
            Self::AlreadyRegistered { tool_name } => {
                write!(f, "Tool '{tool_name}' is already registered")
            }
            Self::CapabilityMismatch {
                tool_name,
                required,
            } => {
                write!(
                    f,
                    "Tool '{tool_name}' capability check failed: {required} required"
                )
            }
        }
    }
}

impl Error for ToolError {}
