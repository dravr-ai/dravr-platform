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
 * How Dravr writes its own name.
 *
 * A proper noun, so it is data rather than copy: the wordmark is identical in
 * all five locales and must never travel through `t()`, where a translator
 * would see an ordinary-looking five-letter string with no way to know it is
 * the product's name. Every surface that draws the lockup — the login hero,
 * the phone's chat header — reads it from here so the five letters exist once.
 */
export const PRODUCT_WORDMARK = 'DRAVR' as const;
