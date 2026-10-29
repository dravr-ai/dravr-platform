// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile client's binding of the shared private-lane hook — a coach's "Only <agent>" turns in a one-athlete room
// ABOUTME: The hook itself lives in @pierre/ui-logic; the phone passes its chat API and keeps the switch's position in AsyncStorage

import AsyncStorage from '@react-native-async-storage/async-storage';
import { createPrivateLaneHooks } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

/**
 * The private lane over the mobile chat API (carnet#1249). The API is reached
 * when the hook calls it, so importing this touches no client. Call it once
 * per room screen and hand the result down: the quote lives in it.
 */
export const { usePrivateLane } = createPrivateLaneHooks({
  chatApi: {
    openAthleteThread: (athleteUserId, groupId) => chatApi.openAthleteThread(athleteUserId, groupId),
    getConversationMessages: (conversationId, options) => chatApi.getConversationMessages(conversationId, options),
    sendTurn: (conversationId, content, options) => chatApi.sendTurn(conversationId, content, options),
  },
  audienceStore: AsyncStorage,
});
