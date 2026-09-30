// ABOUTME: Error code constants for JSON-RPC and MCP protocol errors
// ABOUTME: Defines standard error codes and corresponding error messages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Error codes for JSON-RPC and MCP protocols

/// Method not found
pub const ERROR_METHOD_NOT_FOUND: i32 = -32601;

/// Invalid parameters
pub const ERROR_INVALID_PARAMS: i32 = -32602;

/// Internal error
pub const ERROR_INTERNAL_ERROR: i32 = -32603;

/// MCP-specific error codes for better diagnostics
pub const ERROR_TOOL_EXECUTION: i32 = -32000; // Server error - tool execution failed
/// MCP resource access failed
pub const ERROR_RESOURCE_ACCESS: i32 = -32001; // Server error - resource access failed
/// MCP authentication failed
pub const ERROR_AUTHENTICATION: i32 = -32002; // Server error - authentication failed
/// MCP `resources/read` of a URI no resource answers to (MCP server/resources,
/// "Error Handling": resource not found is -32002)
pub const ERROR_RESOURCE_NOT_FOUND: i32 = -32002;
/// MCP authorization failed (insufficient permissions)
pub const ERROR_AUTHORIZATION: i32 = -32003; // Server error - authorization failed
/// Data serialization/deserialization failed
pub const ERROR_SERIALIZATION: i32 = -32004; // Server error - data serialization failed

/// Rate limit exceeded (daily or weekly quota exhausted)
pub const ERROR_RATE_LIMIT_EXCEEDED: i32 = -32029; // Server error - rate limit exceeded
