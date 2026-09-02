// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The shape a shared module uses to hand display text to a client that has a locale
// ABOUTME: A key and its params, never a finished sentence — the module has no language of its own

/**
 * Text a shared module wants shown, expressed as the catalogue key that
 * carries it.
 *
 * Modules under `packages/` are imported by both clients and have no locale:
 * whatever wording they return is rendered verbatim. Returning English is how
 * the chat progress line and the quota banner came to read English under
 * French chrome (carnet#206, carnet#207), so they return this instead and the
 * component translates at the edge, where the athlete's language is known.
 */
export interface TranslatableText {
  /** Dotted catalogue key, e.g. `chat.status.generatingResponse`. */
  key: string;
  /** Interpolation values for that key, absent when it takes none. */
  params?: Record<string, string | number>;
}

/** Translate a `TranslatableText`, as a client's `t` does. */
export type Translate = (key: string, params?: Record<string, string | number>) => string;
