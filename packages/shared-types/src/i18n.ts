// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The live string catalogue as GET /api/i18n/{locale} serves it
// ABOUTME: One shape for the api-client that fetches it and the i18n package that applies it

/** One locale's catalogue: every key resolved, plus the digest the server revalidates on. */
export interface I18nBundle {
  /** BCP-47 short code the strings were resolved for. */
  locale: string;
  /** Digest of `strings`; sent back as `If-None-Match` so an unchanged catalogue is a 304. */
  etag: string;
  /** Dotted catalogue key to its text, the fallback chain already applied. */
  strings: Record<string, string>;
}

/** What a bundle fetch resolves to: a fresh catalogue, or word that the one held is current. */
export type I18nBundleResult = { status: 'fresh'; bundle: I18nBundle } | { status: 'unchanged' };
