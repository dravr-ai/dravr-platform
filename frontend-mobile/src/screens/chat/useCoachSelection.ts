// ABOUTME: Hook for managing coach selection and starting coach conversations
// ABOUTME: Handles coach loading, selection logic, and auto-sending initial messages

import React, { useState, useCallback } from 'react';
import { Alert } from 'react-native';
import { chatApi, coachesApi } from '../../services/api';
import { extractErrorMessage } from '../../utils/errorMessages';
import type { Coach, Message, Conversation } from '../../types';
import type { ReplyBlock } from '@pierre/shared-types';
import type { CreateConversationParams } from './useConversations';

export interface CoachSelectionState {
  coaches: Coach[];
  pendingCoachAction: { coach: Coach } | null;
  error: string | null;
}

export interface CoachSelectionActions {
  loadCoaches: () => Promise<void>;
  handleCoachSelect: (
    coach: Coach,
    options: {
      isSending: boolean;
      hasConnectedProvider: () => boolean;
      selectedProvider: string | null;
      connectedProviders: { provider: string; connected: boolean }[];
      setSelectedProvider: (provider: string | null) => void;
      setProviderModalVisible: (visible: boolean) => void;
      startCoachConversation: (coach: Coach) => Promise<void>;
    }
  ) => Promise<void>;
  startCoachConversation: (
    coach: Coach,
    options: {
      createConversation: (params: CreateConversationParams) => Promise<Conversation>;
      setMessages: React.Dispatch<React.SetStateAction<Message[]>>;
      setIsSending: (sending: boolean) => void;
      scrollToBottom: () => void;
      setMessageBlocks?: React.Dispatch<React.SetStateAction<Record<string, ReplyBlock[]>>>;
    }
  ) => Promise<void>;
  setPendingCoachAction: (action: { coach: Coach } | null) => void;
  clearPendingCoachAction: () => void;
}

export function useCoachSelection(): CoachSelectionState & CoachSelectionActions {
  const [coaches, setCoaches] = useState<Coach[]>([]);
  const [pendingCoachAction, setPendingCoachAction] = useState<{ coach: Coach } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadCoaches = useCallback(async () => {
    try {
      setError(null);
      const response = await coachesApi.list();
      const sorted = [...response.coaches].sort((a, b) => {
        if (a.is_favorite !== b.is_favorite) {
          return a.is_favorite ? -1 : 1;
        }
        return b.use_count - a.use_count;
      });
      setCoaches(sorted);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to load coaches';
      setError(errorMessage);
      console.error('Failed to load coaches:', err);
    }
  }, []);

  const handleCoachSelect = useCallback(async (
    coach: Coach,
    options: {
      isSending: boolean;
      hasConnectedProvider: () => boolean;
      selectedProvider: string | null;
      connectedProviders: { provider: string; connected: boolean }[];
      setSelectedProvider: (provider: string | null) => void;
      setProviderModalVisible: (visible: boolean) => void;
      startCoachConversation: (coach: Coach) => Promise<void>;
    }
  ) => {
    if (options.isSending) return;

    // Check if we have a cached provider that is still connected
    if (options.selectedProvider) {
      const cachedProvider = options.connectedProviders.find(
        p => p.provider === options.selectedProvider && p.connected
      );
      if (cachedProvider) {
        await options.startCoachConversation(coach);
        return;
      }
      options.setSelectedProvider(null);
    }

    // Check if any provider is connected
    if (options.hasConnectedProvider()) {
      const firstConnected = options.connectedProviders.find(p => p.connected);
      if (firstConnected) {
        options.setSelectedProvider(firstConnected.provider);
      }
      await options.startCoachConversation(coach);
      return;
    }

    // No providers connected - show modal
    setPendingCoachAction({ coach });
    options.setProviderModalVisible(true);
  }, []);

  const startCoachConversation = useCallback(async (
    coach: Coach,
    options: {
      createConversation: (params: CreateConversationParams) => Promise<Conversation>;
      setMessages: React.Dispatch<React.SetStateAction<Message[]>>;
      setIsSending: (sending: boolean) => void;
      scrollToBottom: () => void;
      setMessageBlocks?: React.Dispatch<React.SetStateAction<Record<string, ReplyBlock[]>>>;
    }
  ) => {
    try {
      options.setIsSending(true);
      setError(null);

      // Record usage (fire-and-forget)
      coachesApi.recordUsage(coach.id);

      const conversation = await options.createConversation({
        title: `Chat with ${coach.title}`,
        coach_id: coach.id,
      });

      const initialMessage = coach.startup_query || `Let's get started with ${coach.title}!`;

      const userMessage: Message = {
        id: `temp-${Date.now()}`,
        role: 'user',
        content: initialMessage,
        created_at: new Date().toISOString(),
      };
      options.setMessages([userMessage]);

      // The reply arrives already decomposed — the server read this surface's
      // render capabilities and decided which pieces get their own block.
      const openingBlocks: ReplyBlock[] = [];
      await chatApi.sendTurn(conversation.id, initialMessage, {
        onBlock: block => {
          openingBlocks.push(block);
        },
        onDone: turn => {
          const assistantId = turn.assistant.message.id;
          if (openingBlocks.length > 0 && assistantId && options.setMessageBlocks) {
            const blocks = [...openingBlocks];
            options.setMessageBlocks(prev => ({ ...prev, [assistantId]: blocks }));
          }

          options.setMessages(prev => {
            const filtered = prev.filter(m => m.id !== userMessage.id);
            const newMessages: Message[] = [];
            if (turn.user_message?.id) {
              newMessages.push(turn.user_message);
            }
            if (assistantId) {
              newMessages.push({
                ...turn.assistant.message,
                model: turn.telemetry.model,
                execution_time_ms: turn.telemetry.execution_time_ms,
              });
            }
            return [...filtered, ...newMessages];
          });
        },
        onError: turnErr => {
          // Through the same formatter every other refusal takes, so a quota
          // 429 still names the limit it hit instead of degrading to the raw
          // HTTP message.
          const message = extractErrorMessage(turnErr, 'Failed to start coach conversation');
          setError(message);
          Alert.alert('Coach Error', message);
        },
      });
      options.scrollToBottom();
    } catch (err) {
      const errorMessage = extractErrorMessage(err, 'Failed to start coach conversation');
      setError(errorMessage);
      Alert.alert('Coach Error', errorMessage);
      console.error('Failed to start coach conversation:', err);
    } finally {
      options.setIsSending(false);
    }
  }, []);

  const clearPendingCoachAction = useCallback(() => {
    setPendingCoachAction(null);
  }, []);

  return {
    coaches,
    pendingCoachAction,
    error,
    loadCoaches,
    handleCoachSelect,
    startCoachConversation,
    setPendingCoachAction,
    clearPendingCoachAction,
  };
}
