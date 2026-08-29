// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Messaging-channel proper nouns the UI renders verbatim
// ABOUTME: Data, not copy: identical in all five locales, never routed through t()

/**
 * How each messaging channel writes its own name.
 *
 * These are trademarks. They do not get translated and they do not vary by
 * locale, which is why they live here rather than in the translation corpus —
 * a translator seeing "Messenger" in a list of strings has no way to know it
 * is a product rather than the ordinary noun.
 *
 * Where the server supplies a channel label it stays authoritative: the
 * onboarding picker reads `display_name` off `/api/messaging/channels/available`
 * and the conversation badge reads `channel.label`. This table is for the
 * settings tab, whose per-channel credential fields are static client config
 * and have no server payload to hang a name on.
 */
export const CHANNEL_BRAND = {
  whatsapp: 'WhatsApp',
  telegram: 'Telegram',
  slack: 'Slack',
  discord: 'Discord',
  messenger: 'Messenger',
} as const;

/** A messaging channel this client knows how to configure. */
export type ChannelBrandId = keyof typeof CHANNEL_BRAND;

/**
 * The display name of each coaching persona.
 *
 * Deliberately untranslated: this string is stored on the account, quoted back
 * inside the coach's own system prompt, and shown in the settings list. If the
 * settings list said "Décontracté" while the stored value was "Casual", the two
 * would stop matching and nobody would be able to tell which persona was
 * active. The tagline and description beside it ARE translated.
 */
export const PERSONA_NAME = {
  casual: 'Casual',
  enthusiast: 'Enthusiast',
  power_athlete: 'Power-athlete',
  coach: 'Coach',
} as const;
