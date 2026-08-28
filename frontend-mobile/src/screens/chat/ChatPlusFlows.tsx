// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The modals the chat "+" actions open — the group-name prompt and the participants sheet
// ABOUTME: Rendered by whoever hosts useChatPlusActions, so the tab bar and the chat screens share one flow

import React from 'react';
import { PromptDialog } from '../../components/ui';
import { ConversationParticipantsModal } from './ConversationParticipantsModal';
import type { ChatPlusFlowState } from './useChatPlusActions';
import { useTranslation } from '@pierre/i18n';

interface ChatPlusFlowsProps {
  flows: ChatPlusFlowState;
}

/**
 * The follow-through of a "+" action.
 *
 * Both modals sit beside the menu that opened them rather than inside it: a
 * modal nested in a closing modal is exactly the sequence iOS drops on the
 * floor, so the host renders these as siblings and the menu only flips state.
 */
export function ChatPlusFlows({ flows }: ChatPlusFlowsProps) {
  const { t } = useTranslation();
  return (
    <>
      <PromptDialog
        visible={flows.groupNamePromptVisible}
        title={t('app.newGroupChat')}
        message="What is this group called?"
        placeholder={t('app.groupNamePlaceholder')}
        submitText={t('app.create')}
        cancelText={t('common.cancel')}
        onSubmit={flows.submitGroupName}
        onCancel={flows.closeGroupNamePrompt}
        testID="new-group-name-dialog"
      />
      <ConversationParticipantsModal
        visible={flows.participantsVisible}
        conversationId={flows.conversationId}
        onClose={flows.closeParticipants}
      />
    </>
  );
}
