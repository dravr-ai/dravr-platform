// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Types for first-party-sign-in.js, so TypeScript callers (bun scripts, specs) import it typed
// ABOUTME: Mirrors the CommonJS exports one to one

export interface FirstPartySignInOptions {
  baseUrl: string;
  email: string;
  password: string;
  clientId?: 'dravr-mobile' | 'dravr-web';
  frontendUrl?: string;
  scope?: string;
}

export interface FirstPartyTokenResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token?: string;
  csrf_token?: string;
  user?: Record<string, unknown>;
  [key: string]: unknown;
}

export declare class FirstPartySignInError extends Error {
  status: number | undefined;
}

export declare function firstPartySignIn(options: FirstPartySignInOptions): Promise<FirstPartyTokenResponse>;

export declare const MOBILE_CLIENT_ID: 'dravr-mobile';
export declare const WEB_CLIENT_ID: 'dravr-web';
