// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat "+" actions — new chat, new group chat, add someone to the open discussion
// ABOUTME: One list behind the tab bar's quick actions and the chat screens' sheet, so the set cannot drift

import { useCallback, useMemo, useState } from 'react';
import { useRouter } from 'expo-router';
import { MessageSquarePlus, UserPlus, Users } from 'lucide-react-native';
import type { LucideIcon } from 'lucide-react-native';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';
import { threadHref } from '../../navigation/routes';
import { useTranslation } from '@pierre/i18n';

/** The three things the chat "+" can do. */
export type ChatPlusActionId = 'new-chat' | 'new-group-chat' | 'add-participant';

/** One row of the "+" menu, wherever it is rendered. */
export interface ChatPlusAction {
  id: ChatPlusActionId;
  label: string;
  icon: LucideIcon;
  onPress: () => void;
}

/**
 * Catalogue keys for the three actions the composer's "+" offers.
 *
 * Keys rather than the words: this module has no locale, and the finished
 * English it used to export was what the sheet rendered under French chrome
 * (carnet#207).
 */
export const NEW_CHAT_LABEL_KEY = 'chat.newChat';
export const NEW_GROUP_CHAT_LABEL_KEY = 'chat.newGroupChat';
export const ADD_PARTICIPANT_LABEL_KEY = 'chat.addSomeoneHint';

/** The modal flows an action opens; rendered by {@link ChatPlusFlows}. */
export interface ChatPlusFlowState {
  /** The conversation "add someone" acts on, or null when none is open. */
  conversationId: string | null;
  groupNamePromptVisible: boolean;
  participantsVisible: boolean;
  openParticipants: () => void;
  closeGroupNamePrompt: () => void;
  closeParticipants: () => void;
  /** Open a fresh thread that creates `name` as a group on its first turn. */
  submitGroupName: (name: string) => void;
}

export interface UseChatPlusActionsResult {
  /** The rows to offer, in order: new chat, new group chat, then add someone when a discussion is open. */
  actions: ChatPlusAction[];
  flows: ChatPlusFlowState;
}

/**
 * The chat "+" affordance, as ChefFamille asked for it: new chat, new group
 * chat, and an easy way to add someone to an existing discussion.
 *
 * "Add someone" appears only with a conversation open — the participants
 * routes need a thread to act on, and a fresh composer has none yet. New chat
 * opens an empty thread. New group chat asks for a name and then opens a
 * thread that sends `/group create <name>`: the command is the one
 * implementation of creating a group, shared with web and with messaging, so
 * the app has no second way to make one.
 */
export function useChatPlusActions(conversationId: string | null): UseChatPlusActionsResult {
  const { t } = useTranslation();
  const router = useRouter();
  const [groupNamePromptVisible, setGroupNamePromptVisible] = useState(false);
  const [participantsVisible, setParticipantsVisible] = useState(false);

  const submitGroupName = useCallback(
    (name: string) => {
      setGroupNamePromptVisible(false);
      const trimmed = name.trim();
      if (!trimmed) return;
      router.push(threadHref(undefined, { send: COMMAND_DRAFTS.groupCreate(trimmed) }));
    },
    [router],
  );

  const actions = useMemo<ChatPlusAction[]>(() => {
    const list: ChatPlusAction[] = [
      {
        id: 'new-chat',
        label: t(NEW_CHAT_LABEL_KEY),
        icon: MessageSquarePlus,
        onPress: () => router.push(threadHref()),
      },
      {
        id: 'new-group-chat',
        label: t(NEW_GROUP_CHAT_LABEL_KEY),
        icon: Users,
        onPress: () => setGroupNamePromptVisible(true),
      },
    ];
    if (conversationId !== null) {
      list.push({
        id: 'add-participant',
        label: t(ADD_PARTICIPANT_LABEL_KEY),
        icon: UserPlus,
        onPress: () => setParticipantsVisible(true),
      });
    }
    return list;
  }, [conversationId, router, t]);

  const flows = useMemo<ChatPlusFlowState>(
    () => ({
      conversationId,
      groupNamePromptVisible,
      participantsVisible,
      openParticipants: () => setParticipantsVisible(true),
      closeGroupNamePrompt: () => setGroupNamePromptVisible(false),
      closeParticipants: () => setParticipantsVisible(false),
      submitGroupName,
    }),
    [conversationId, groupNamePromptVisible, participantsVisible, submitGroupName],
  );

  return { actions, flows };
}
