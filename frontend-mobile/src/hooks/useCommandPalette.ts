// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile composer's slash-command palette — the shared palette, answering for the routed conversation
// ABOUTME: The conversation comes from the route, the one part of the palette that is mobile's own

import { useLocalSearchParams } from 'expo-router';
import {
  createCommandPaletteHook,
  type UseCommandPaletteOptions,
  type UseCommandPaletteResult,
} from '@pierre/ui-logic';
import { chatApi } from '../services/api';
import { NEW_CONVERSATION_ID } from '../navigation/routes';

const useSharedCommandPalette = createCommandPaletteHook(chatApi);

/**
 * The slash-command palette for the conversation on screen.
 *
 * The conversation is read from the route rather than taken as an argument.
 * `conversationId` is the same param the chat screen selects its conversation
 * from, so the palette and the transcript always describe the same thread, and
 * the composer needs no new prop threaded through to get it. The sentinel
 * `'new'` means no conversation yet, which the server reads as "answer for the
 * caller's own memberships" — the right answer for a thread with no group.
 */
export function useCommandPalette(
  options: Omit<UseCommandPaletteOptions, 'conversationId'>,
): UseCommandPaletteResult {
  const params = useLocalSearchParams<{ conversationId?: string }>();
  const conversationId =
    params?.conversationId && params.conversationId !== NEW_CONVERSATION_ID
      ? params.conversationId
      : undefined;
  return useSharedCommandPalette({ ...options, conversationId });
}
