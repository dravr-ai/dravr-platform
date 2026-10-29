// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web client's binding of the shared private-lane hook — a coach's "Only <agent>" turns in a one-athlete room
// ABOUTME: The hook itself lives in @pierre/ui-logic, one definition for web and mobile (carnet#1249)

import { createPrivateLaneHooks, type AudienceStore } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

/**
 * Where the Everyone / Only <agent> switch is remembered: this browser's
 * `localStorage`. Reached on each call, so a browser that refuses storage (a
 * private window) throws inside the call, where the hook absorbs it.
 */
const audienceStore: AudienceStore = {
  getItem: (key) => window.localStorage.getItem(key),
  setItem: (key, value) => window.localStorage.setItem(key, value),
};

/**
 * The private-lane hook over the web chat API. The API is reached when the
 * hook calls it, so importing this touches no client.
 */
export const { usePrivateLane } = createPrivateLaneHooks({
  chatApi: {
    openAthleteThread: (athleteUserId, groupId) => chatApi.openAthleteThread(athleteUserId, groupId),
    getConversationMessages: (conversationId, window) => chatApi.getConversationMessages(conversationId, window),
    sendTurn: (conversationId, content, options) => chatApi.sendTurn(conversationId, content, options),
  },
  audienceStore,
});
