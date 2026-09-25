// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main entry point for Dravr MCP Client TypeScript SDK
// ABOUTME: Re-exports MCP client and configuration for programmatic integration
//
// Licensed under either of Apache License, Version 2.0 or MIT License at your option.
// Copyright (c) 2026 dravr.ai

/**
 * Dravr MCP Client SDK
 *
 * Export the main MCP client implementation for programmatic use
 */

export { PierreMcpClient, BridgeConfig } from './mcp-bridge';

/**
 * Export the stateless MCP client the bridge uses to reach the Dravr server over
 * Streamable HTTP, protocol revision 2026-07-28, for programmatic use without the
 * stdio bridge around it.
 */
export {
  McpHttpClient,
  McpHttpError,
  McpRpcError,
  McpProtocolError,
  McpTaskCancelledError,
  encodeHeaderValue,
  MCP_PROTOCOL_VERSION,
  TASKS_EXTENSION_ID,
  META_PROTOCOL_VERSION,
  META_CLIENT_INFO,
  META_CLIENT_CAPABILITIES,
  META_SERVER_INFO,
  RPC_METHOD_NOT_FOUND,
  RPC_INVALID_PARAMS,
  RPC_HEADER_MISMATCH,
  RPC_MISSING_REQUIRED_CLIENT_CAPABILITY,
  RPC_UNSUPPORTED_PROTOCOL_VERSION,
  type McpHttpClientOptions,
  type CallToolOptions,
  type RequestOptions,
  type DiscoverResult,
  type ServerCapabilities,
  type ClientCapabilities,
  type Implementation,
  type McpToolDefinition,
  type McpListToolsResult,
  type McpCallToolResult,
  type McpContentBlock,
  type McpTask,
  type McpTaskState,
  type TaskStatus,
  type McpProgress,
  type McpNotification,
  type JsonRpcErrorShape,
} from './mcp-http-client';

/**
 * Export the OAuth client provider used by the bridge for programmatic embedding
 * and for testing the non-interactive (browser-disabled) authorization guard.
 */
export { PierreOAuthClientProvider } from './oauth-session-manager';

/**
 * Export structured error types for typed error handling
 */
export { PierreError, PierreErrorCode } from './errors';

/**
 * Export all TypeScript type definitions for Dravr MCP tools
 *
 * These types are auto-generated from server tool schemas.
 * To regenerate: bun run generate-types
 */
export * from './types';

/**
 * Export response validation utilities
 *
 * Use these to configure validation behavior, check validation stats,
 * or manually validate tool responses.
 */
export {
  validateMcpToolResponse,
  registerToolOutputSchemas,
  hasResponseSchema,
  configureValidator,
  getValidatorConfig,
  createValidatedToolCall,
  isValidResponse,
  getValidationStats,
  resetValidationStats,
  validateWithStats,
  type ResponseValidatorConfig,
  type ValidatedToolResult,
  type ValidationStats,
  type ToolWithOutputSchema,
} from './response-validator';

/**
 * Export token storage utilities for OAuth tokens and credentials
 *
 * createSecureStorage prefers the OS keychain. EncryptedFileStorage is the fallback
 * used when no keychain is available: it stores tokens at ~/.pierre-mcp-tokens.enc and
 * its protection is the owner-only (0600) file mode, not its AES layer, whose key is
 * derived from non-secret machine data.
 */
export {
  createSecureStorage,
  EncryptedFileStorage,
  type SecureTokenStorage,
} from './secure-storage';