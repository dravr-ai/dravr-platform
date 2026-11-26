// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One of the athlete workspace's threads on the phone — the coach's plan thread or the athlete's room — drawn by the chat screen's own ChatThread
// ABOUTME: The host reads the conversation when it changes, sends into it and moves its read marker; the workspace decides which conversation it is (carnet#1818)

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { TextInput } from 'react-native';
import type { TurnSendOptions } from '@pierre/shared-types';
import { usePlanThreadDraft } from '../../../hooks/useCoachPlan';
import { trackMobile } from '../../../services/analytics';
import { ChatThread } from '../../chat/ChatThread';
import { useMarkConversationRead } from '../../chat/useMarkConversationRead';
import type { WorkspacePlanSurface } from '../../chat/PlanChangeEntry';
import { useMessages } from '../../chat/useMessages';
import { useProviderStatus } from '../../chat/useProviderStatus';
import { useUsageStatus } from '../../chat/useUsageStatus';

interface WorkspaceThreadProps {
  /** The conversation the tab draws. */
  conversationId: string;
  /** The room's group, whose rows the transcript weaves in; `null` for the plan thread. */
  groupId: string | null;
  /** What the conversation is called, for the verdict sheet. */
  conversationTitle: string;
  /** The athlete, when the thread is the coach's plan thread on them. */
  planSubjectName?: string | null;
  /** The room's name, when the thread is the coach's conversation in the athlete's room. */
  coachRoomName?: string | null;
  /** The plan thread's cards lead back to the plan tab. */
  planSurface?: WorkspacePlanSurface | null;
  /** A line the workspace hands the thread to send once it is read — « Rédiger le plan ». */
  pendingSend?: string | null;
  onPendingSendConsumed?: () => void;
}

/**
 * A workspace tab's thread: the chat screen's transcript and composer over
 * one conversation that already exists. The conversation is read when it
 * changes and only then — `loadMessages` changes whenever the transcript
 * grows, so it is reached through a ref, as an activity's thread does. A
 * `/reset` that moves the thread on is not followed: the workspace decides
 * which conversation each tab draws, as on the web.
 */
export function WorkspaceThread({
  conversationId,
  groupId,
  conversationTitle,
  planSubjectName = null,
  coachRoomName = null,
  planSurface = null,
  pendingSend = null,
  onPendingSendConsumed,
}: WorkspaceThreadProps) {
  const messagesHook = useMessages();
  const usageStatus = useUsageStatus();
  const providerStatus = useProviderStatus();
  const planDraftRead = usePlanThreadDraft(conversationId, planSubjectName !== null);
  const inputRef = useRef<TextInput>(null);
  const [inputText, setInputText] = useState('');
  // The conversation whose transcript has been read, so a line handed to the
  // thread is sent after the read and is never painted over by it.
  const [readId, setReadId] = useState<string | null>(null);

  const { loadProviderStatus } = providerStatus;
  useEffect(() => {
    void loadProviderStatus();
  }, [loadProviderStatus]);

  const { loadMessages } = messagesHook;
  const threadReads = useRef(loadMessages);
  threadReads.current = loadMessages;
  useEffect(() => {
    let current = true;
    void threadReads.current(conversationId, groupId).then(() => {
      if (current) setReadId(conversationId);
    });
    return () => {
      current = false;
    };
  }, [conversationId, groupId]);

  // Reading is looking, as on the chat screen: the newest row of the coach's
  // own conversation moves its read marker — a room's rows are not theirs.
  const { messages } = messagesHook;
  const lastMessageId = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      if (!messages[i].room) return messages[i].id;
    }
    return null;
  }, [messages]);
  useMarkConversationRead({ conversationId: readId === conversationId ? conversationId : null, lastMessageId });

  const { isSending, sendTurn } = messagesHook;
  const { invalidate } = usageStatus;
  const sendText = useCallback(
    async (text: string, options?: TurnSendOptions) => {
      const trimmed = text.trim();
      if (!trimmed || isSending) return;
      try {
        trackMobile({ name: 'feature_engaged', props: { feature: 'chat_message_sent' } });
        await sendTurn(conversationId, trimmed, options);
      } finally {
        invalidate();
      }
    },
    [conversationId, invalidate, isSending, sendTurn],
  );

  useEffect(() => {
    if (pendingSend === null || readId !== conversationId) return;
    onPendingSendConsumed?.();
    void sendText(pendingSend);
  }, [pendingSend, readId, conversationId, onPendingSendConsumed, sendText]);

  return (
    <ChatThread
      conversationId={conversationId}
      conversationTitle={conversationTitle}
      messagesHook={messagesHook}
      usageStatus={usageStatus}
      providerStatus={providerStatus}
      isLoading={readId !== conversationId}
      inputText={inputText}
      onChangeInputText={setInputText}
      inputRef={inputRef}
      sendText={sendText}
      planSubjectName={planSubjectName}
      coachRoomName={coachRoomName}
      planDraftRead={planDraftRead}
      planSurface={planSurface}
    />
  );
}
