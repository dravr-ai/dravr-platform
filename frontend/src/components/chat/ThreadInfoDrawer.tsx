// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The open thread's info drawer as Home and the Groups tab both mount it — group info, agent info or the plain thread's controls
// ABOUTME: Its actions reach the thread, the list and the router; whether it is open is useThreadInfo's

import type { Conversation } from '@pierre/shared-types';
import { useConversationMutations } from '../../hooks/useConversationList';
import { CONNECTIONS_ROUTE } from '../../constants/surfaceLayout';
import ConversationInfoPanel from './ConversationInfoPanel';
import type { ThreadInfoState } from './useThreadInfo';

interface ThreadInfoDrawerProps {
  info: ThreadInfoState;
  /** The open thread's list row; the drawer draws nothing without one. */
  conversation: Conversation | null;
  /** Send a command line into the thread — `/agent remove`, a group action. */
  onSendCommand: (text: string) => void;
  onSelectConversation: (id: string | null) => void;
  /** Dashboard route navigator; editing an agent leaves for `discover/<agentId>`. */
  onNavigate?: (route: string) => void;
}

/** The info drawer over the open thread, wired to the thread, the list and the router. */
export default function ThreadInfoDrawer({
  info,
  conversation,
  onSendCommand,
  onSelectConversation,
  onNavigate,
}: ThreadInfoDrawerProps) {
  const { rename, remove } = useConversationMutations();
  if (!info.open || !conversation) return null;

  /** The thread is gone — deleted, or left with the group it belonged to. */
  const handleThreadGone = () => {
    info.close();
    onSelectConversation(null);
  };

  return (
    <ConversationInfoPanel
      conversation={conversation}
      openParticipants={info.opensParticipants}
      onClose={info.close}
      onSendCommand={(text) => {
        info.close();
        onSendCommand(text);
      }}
      onEditCoach={(agentId) => {
        info.close();
        onNavigate?.(`discover/${encodeURIComponent(agentId)}`);
      }}
      onRename={(title) => {
        void rename(conversation.id, title);
        info.close();
      }}
      onDelete={() => {
        void remove(conversation.id).then(handleThreadGone);
      }}
      onThreadGone={handleThreadGone}
      onOpenConversation={(conversationId) => {
        info.close();
        onSelectConversation(conversationId);
      }}
      onOpenConnections={
        onNavigate
          ? () => {
              info.close();
              onNavigate(CONNECTIONS_ROUTE);
            }
          : undefined
      }
    />
  );
}
