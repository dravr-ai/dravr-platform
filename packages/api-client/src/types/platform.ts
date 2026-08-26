// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Platform abstraction interfaces for HTTP client and auth storage
// ABOUTME: Allows the same API logic to work on web (axios/localStorage) and mobile (axios/AsyncStorage)

import type { AxiosInstance } from 'axios';

/**
 * Platform-specific HTTP client configuration.
 * Abstracts differences between web and mobile axios setup.
 */
export interface HttpClientConfig {
  /** Base URL for API requests */
  baseURL: string;
  /** Default timeout in milliseconds */
  timeout?: number;
  /** Whether to include credentials (cookies) - web only */
  withCredentials?: boolean;
  /** Default headers to include */
  defaultHeaders?: Record<string, string>;
}

/**
 * Platform-specific authentication storage.
 * Abstracts localStorage (web) vs AsyncStorage (mobile).
 * All methods are async for mobile compatibility.
 */
export interface AuthStorage {
  /** Get JWT token */
  getToken(): Promise<string | null>;
  /** Store JWT token */
  setToken(token: string): Promise<void>;
  /** Remove JWT token */
  removeToken(): Promise<void>;
  /** Get CSRF token */
  getCsrfToken(): Promise<string | null>;
  /** Store CSRF token */
  setCsrfToken(token: string | null): Promise<void>;
  /** Get stored user data */
  getUser<T>(): Promise<T | null>;
  /** Store user data */
  setUser<T>(user: T): Promise<void>;
  /** Get refresh token */
  getRefreshToken(): Promise<string | null>;
  /** Store refresh token */
  setRefreshToken(token: string): Promise<void>;
  /** Clear all auth data */
  clear(): Promise<void>;
}

/**
 * Platform-specific auth failure handler.
 * Abstracts window events (web) vs event emitter (mobile).
 */
export interface AuthFailureHandler {
  /** Called when authentication fails (401 response) */
  onAuthFailure(): void;
  /** Subscribe to auth failure events */
  subscribe(listener: () => void): () => void;
}

/**
 * How this platform reads a response body.
 *
 * The one place the two runtimes genuinely differ: a browser hands back a
 * `ReadableStream` the client can render from as bytes land, while React
 * Native's `fetch` rides XHR and leaves `Response.body` undefined, so the
 * body is only readable once complete. Both yield the same thing — the body
 * as text — so the turn parser above them has a single code path and never
 * asks which platform it is on.
 *
 * A body that arrives in one piece is not a degraded turn: the terminal frame
 * is the same either way, and only the intermediate deltas (which exist on
 * one provider branch) are lost to the wait.
 */
export type ResponseBodyReader = (response: Response) => AsyncIterable<string>;

/**
 * Complete platform adapter combining all platform-specific concerns.
 */
export interface PlatformAdapter {
  /** HTTP client configuration */
  httpConfig: HttpClientConfig;
  /** Auth storage implementation */
  authStorage: AuthStorage;
  /** Auth failure handler */
  authFailure: AuthFailureHandler;
  /** How this platform reads a streaming response body. */
  readBody: ResponseBodyReader;
  /**
   * How this platform carries its credentials on a turn request.
   *
   * Web authenticates with an httpOnly session cookie, so the fetch must
   * include it; mobile sends a bearer header and has no cookie jar. Declared
   * here rather than branched on inside the transport, so the send path has
   * one shape for every surface.
   */
  turnCredentials: RequestCredentials;
  /** Platform identifier, and the `X-Client-Platform` header's value. */
  platform: 'web' | 'mobile';
}

/**
 * Options for creating the API client.
 */
export interface ApiClientOptions {
  /** Platform adapter providing platform-specific implementations */
  adapter: PlatformAdapter;
  /** Optional axios instance (for testing/customization) */
  axiosInstance?: AxiosInstance;
}

/**
 * Standard API response metadata.
 */
export interface ApiMetadata {
  timestamp: string;
  api_version: string;
}

/**
 * Paginated response with cursor.
 */
export interface CursorPaginatedResponse<T> {
  items: T[];
  next_cursor: string | null;
  has_more: boolean;
  metadata: ApiMetadata;
}

/**
 * Paginated response with offset.
 */
export interface OffsetPaginatedResponse<T> {
  items: T[];
  total: number;
  offset: number;
  limit: number;
  metadata: ApiMetadata;
}
