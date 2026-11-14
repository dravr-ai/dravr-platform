// ABOUTME: The chat-app offer outside the wizard — the onboarding record its answer is kept on, and the app it names
// ABOUTME: Shared by web and mobile, so « Pas maintenant » said on one device holds on the other (carnet#1525)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { PersistedOnboardingStep } from './onboarding';

/**
 * The server onboarding record the chat-app offer is answered on.
 *
 * Linking a chat app (Telegram, WhatsApp, …) was the wizard's last two
 * screens until it moved out of it: an athlete now meets it as a Getting
 * started row on Home, a coach as one card in their group's thread. The answer
 * still lands on the record those screens wrote — `skipped` for « Pas
 * maintenant », `complete` once an app is linked from the offer — so it holds
 * on every device, and an account that skipped the old screen is not asked
 * again.
 */
export const CHAT_APP_OFFER_STEP_ID = 'messaging_configure';

/** Whether the person has answered the chat-app offer, on any surface or device. */
export function chatAppOfferAnswered(steps: PersistedOnboardingStep[] | undefined): boolean {
  return (steps ?? []).some(
    (s) => s.step_id === CHAT_APP_OFFER_STEP_ID && (s.status === 'complete' || s.status === 'skipped'),
  );
}

/**
 * The app an offer names when it can name only one: the one the server
 * recommends (Telegram, the most complete), else the first it lists. The
 * sheet the offer opens still lets the person pick another.
 */
export function offeredChatApp<T extends { recommended: boolean }>(channels: readonly T[]): T | undefined {
  return channels.find((c) => c.recommended) ?? channels[0];
}
