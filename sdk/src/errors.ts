// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Structured error types for Dravr MCP Client SDK
// ABOUTME: Provides typed error codes and PierreError class for consistent error handling

/**
 * Error codes for categorizing Dravr SDK errors
 */
export enum PierreErrorCode {
  NETWORK_ERROR = 'NETWORK_ERROR',
  AUTH_ERROR = 'AUTH_ERROR',
  CONFIG_ERROR = 'CONFIG_ERROR',
  STORAGE_ERROR = 'STORAGE_ERROR',
  TIMEOUT_ERROR = 'TIMEOUT_ERROR',
  VALIDATION_ERROR = 'VALIDATION_ERROR',
  PROVIDER_ERROR = 'PROVIDER_ERROR',
}

/**
 * Structured error class for Dravr SDK operations
 * Extends Error with a typed error code and optional cause
 */
export class PierreError extends Error {
  constructor(
    public readonly code: PierreErrorCode,
    message: string,
    public readonly cause?: Error,
  ) {
    super(message);
    this.name = 'PierreError';
  }
}

/**
 * An OAuth 2.0 error answer from Dravr's authorization server: an authorization response
 * (RFC 6749 section 4.1.2.1), a token response (section 5.2) or a client registration
 * response (RFC 7591 section 3.2.2). It carries the server's `error` code, so a caller
 * reacts to the refusal the server named rather than to the wording of a message.
 */
export class OAuthServerError extends PierreError {
  constructor(
    public readonly oauthError: string,
    message: string,
  ) {
    super(PierreErrorCode.AUTH_ERROR, message);
    this.name = 'OAuthServerError';
  }
}
