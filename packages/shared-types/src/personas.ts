// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The persona card GET /api/personas serves — the « Style de coaching » surface's content
// ABOUTME: Mirrors pierre_services::personas::PersonaCard; the server renders it from the live contract registry

/**
 * One localized rule the persona's contract imposes.
 *
 * `key` is the rule's identity across locales (`persona.rule.*`); `text` is
 * the sentence the server already rendered, with the contract's own numbers
 * interpolated — a word cap the client has no way to know.
 */
export interface PersonaRule {
  key: string;
  text: string;
}

/**
 * One card of the coaching-style picker.
 *
 * Every field is the server's: the client used to hold four hand-written
 * cards, each with a tagline, a blurb and up to two bullets in five locales,
 * describing contracts it could not see. When a contract changed — a word cap,
 * a rule added, strict mode switched on — the cards went on saying whatever
 * they had said before.
 */
export interface PersonaCard {
  /** Canonical `snake_case` slug — the value stored on the account. */
  slug: string;
  /** Brand name, deliberately untranslated so it matches the stored value. */
  display_name: string;
  /** One localized line on how this persona speaks. */
  summary: string;
  /** Every rule the flattened contract sets, in declaration order. */
  rules: PersonaRule[];
  /** `verified` when the contract runs strict mode, `advisory` otherwise. */
  enforcement: string;
  /** The localized word for {@link enforcement}. */
  enforcement_label: string;
}

/** Response envelope for `GET /api/personas`. */
export interface PersonasResponse {
  personas: PersonaCard[];
}
