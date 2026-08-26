// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat "+" actions — new chat, new group chat, add someone to the open discussion
// ABOUTME: One list behind the tab bar's quick actions and the chat screens' sheet, so the set cannot drift

import { useCallback, useMemo, useState } from 'react';
import { useRouter } from 'expo-router';
import { MessageSquarePlus, UserPlus, Users } from 'lucide-react-native';
import type { LucideIcon } from 'lucide-react-native';
import { threadHref } from '../../navigation/routes';
import { useStartGroupConversation } from '../../hooks/useStartGroupConversation';
import type { GroupSummary } from '../../types';

/** The three things the chat "+" can do. */
export type ChatPlusActionId = 'new-chat' | 'new-group-chat' | 'add-participant';

/** One row of the "+" menu, wherever it is rendered. */
export interface ChatPlusAction {
  id: ChatPlusActionId;
  label: string;
  icon: LucideIcon;
  onPress: () => void;
}

export const NEW_CHAT_LABEL = 'New chat';
export const NEW_GROUP_CHAT_LABEL = 'New group chat';
export const ADD_PARTICIPANT_LABEL = 'Add someone to this discussion';

/** The modal flows an action opens; rendered by {@link ChatPlusFlows}. */
export interface ChatPlusFlowState {
  /** The conversation "add someone" acts on, or null when none is open. */
  conversationId: string | null;
  groupPickerVisible: boolean;
  participantsVisible: boolean;
  isStartingGroupChat: boolean;
  openParticipants: () => void;
  closeGroupPicker: () => void;
  closeParticipants: () => void;
  pickGroup: (group: GroupSummary) => void;
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
 * routes need a thread to act on, and a fresh composer has none yet. New
 * chat opens an empty thread; new group chat asks which room first, then
 * takes the same path the group screen's own button takes.
 */
export function useChatPlusActions(conversationId: string | null): UseChatPlusActionsResult {
  const router = useRouter();
  const { start, isStarting } = useStartGroupConversation();
  const [groupPickerVisible, setGroupPickerVisible] = useState(false);
  const [participantsVisible, setParticipantsVisible] = useState(false);

  const pickGroup = useCallback(
    (group: GroupSummary) => {
      setGroupPickerVisible(false);
      void start(group);
    },
    [start],
  );

  const actions = useMemo<ChatPlusAction[]>(() => {
    const list: ChatPlusAction[] = [
      {
        id: 'new-chat',
        label: NEW_CHAT_LABEL,
        icon: MessageSquarePlus,
        onPress: () => router.push(threadHref()),
      },
      {
        id: 'new-group-chat',
        label: NEW_GROUP_CHAT_LABEL,
        icon: Users,
        onPress: () => setGroupPickerVisible(true),
      },
    ];
    if (conversationId !== null) {
      list.push({
        id: 'add-participant',
        label: ADD_PARTICIPANT_LABEL,
        icon: UserPlus,
        onPress: () => setParticipantsVisible(true),
      });
    }
    return list;
  }, [conversationId, router]);

  const flows = useMemo<ChatPlusFlowState>(
    () => ({
      conversationId,
      groupPickerVisible,
      participantsVisible,
      isStartingGroupChat: isStarting,
      openParticipants: () => setParticipantsVisible(true),
      closeGroupPicker: () => setGroupPickerVisible(false),
      closeParticipants: () => setParticipantsVisible(false),
      pickGroup,
    }),
    [conversationId, groupPickerVisible, participantsVisible, isStarting, pickGroup],
  );

  return { actions, flows };
}
