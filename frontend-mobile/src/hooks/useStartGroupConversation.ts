// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Starts a group-scoped conversation and opens its thread — the one path behind every "group chat" entry
// ABOUTME: The group screen's button and the chat "+" both go through here, so neither can drop group_id

import { useCallback, useState } from 'react';
import { Alert } from 'react-native';
import { useRouter } from 'expo-router';
import { chatApi } from '../services/api';
import { threadHref } from '../navigation/routes';
import type { Conversation } from '../types';

/** What a group must carry to open a conversation scoped to it. */
export interface GroupConversationTarget {
  id: string;
  name: string;
  coach_id: string;
}

/**
 * Create a conversation bound to a coaching group and navigate into it.
 *
 * `group_id` is the field that turns on group context and the peer-grounding
 * stage server-side; the conversation is titled with the group's name so the
 * thread header names the room. The server verifies membership and refuses a
 * non-member, which surfaces here as the alert.
 */
export function useStartGroupConversation() {
  const router = useRouter();
  const [isStarting, setIsStarting] = useState(false);

  const start = useCallback(
    async (group: GroupConversationTarget): Promise<Conversation | null> => {
      setIsStarting(true);
      try {
        const conversation = await chatApi.createConversation({
          title: group.name,
          coach_id: group.coach_id,
          group_id: group.id,
        });
        router.push(threadHref(conversation.id));
        return conversation;
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Failed to open the group chat';
        Alert.alert('Error', msg);
        return null;
      } finally {
        setIsStarting(false);
      }
    },
    [router],
  );

  return { start, isStarting };
}
