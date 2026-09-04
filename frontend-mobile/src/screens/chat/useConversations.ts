// ABOUTME: Hook for managing chat conversations CRUD operations and state
// ABOUTME: Handles loading, creating, updating, deleting, and selecting conversations

import React, { useState, useCallback, useRef } from 'react';
import { Alert } from 'react-native';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { describeApiError } from '@pierre/ui-logic';
import { chatApi } from '../../services/api';
import type { Conversation } from '../../types';
import { useTranslation } from '@pierre/i18n';

export interface ConversationsState {
  conversations: Conversation[];
  currentConversation: Conversation | null;
  isLoading: boolean;
  error: string | null;
}

/**
 * What a caller may attach to a new conversation.
 *
 * `group_id` scopes the conversation to a coaching group: the server checks
 * the caller's membership, then gates group context and the peer-grounding
 * fabrication stage on it. Leaving it off makes the conversation a personal
 * 1:1 chat, whatever coach persona it carries.
 */
export interface CreateConversationParams {
  title: string;
  coach_id?: string;
  group_id?: string;
}

export interface ConversationsActions {
  loadConversations: () => Promise<void>;
  setCurrentConversation: (conversation: Conversation | null) => void;
  createConversation: (params: CreateConversationParams) => Promise<Conversation>;
  /**
   * Open `conversationId`, reading the row from the server first.
   *
   * For a thread the server forged rather than this screen: `/reset` archives
   * the current one and continues on a fresh row nothing local has yet.
   * Answers whether it resolved.
   */
  switchToConversation: (conversationId: string) => Promise<boolean>;
  deleteConversation: (conversationId: string) => Promise<void>;
  renameConversation: (conversationId: string, newTitle: string) => Promise<void>;
  justCreatedConversationRef: React.MutableRefObject<string | null>;
}

export function useConversations(): ConversationsState & ConversationsActions {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [currentConversation, setCurrentConversation] = useState<Conversation | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const justCreatedConversationRef = useRef<string | null>(null);

  // The unified list is a React Query cache the conversation screen and the
  // chat tab's badge both read. A thread created, renamed or deleted from the
  // open transcript changes that list, so it is re-read rather than left to
  // drift until the next focus.
  const invalidateConversationList = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
  }, [queryClient]);

  const loadConversations = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const response = await chatApi.getConversations();
      const seen = new Set<string>();
      const deduplicated = (response.conversations || []).filter((conv: { id: string }) => {
        if (seen.has(conv.id)) return false;
        seen.add(conv.id);
        return true;
      });
      const sorted = deduplicated.sort((a: { updated_at?: string }, b: { updated_at?: string }) => {
        const dateA = a.updated_at ? new Date(a.updated_at).getTime() : 0;
        const dateB = b.updated_at ? new Date(b.updated_at).getTime() : 0;
        return dateB - dateA;
      });
      setConversations(sorted);
    } catch (err) {
      const errorMessage =
        err instanceof Error ? err.message : t('app.failedLoadConversations');
      setError(errorMessage);
      console.error('Failed to load conversations:', err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const createConversation = useCallback(async (params: CreateConversationParams): Promise<Conversation> => {
    try {
      setError(null);
      const conversation = await chatApi.createConversation(params);
      if (!conversation || !conversation.id) {
        throw new Error(t('app.invalidConversationResponse'));
      }
      setConversations(prev => [conversation, ...prev]);
      justCreatedConversationRef.current = conversation.id;
      setCurrentConversation(conversation);
      invalidateConversationList();
      return conversation;
    } catch (err) {
      const errorMessage = describeApiError(err, {
        t,
        fallbackKey: 'app.failedCreateConversation',
      });
      setError(errorMessage);
      console.error('Failed to create conversation:', err);
      throw err;
    }
  }, [invalidateConversationList]);

  const deleteConversation = useCallback(async (conversationId: string) => {
    try {
      setError(null);
      await chatApi.deleteConversation(conversationId);
      setConversations(prev => prev.filter(c => c.id !== conversationId));
      if (currentConversation?.id === conversationId) {
        setCurrentConversation(null);
      }
      invalidateConversationList();
    } catch (err) {
      const errorMessage =
        err instanceof Error ? err.message : t('app.failedDeleteConversation');
      setError(errorMessage);
      Alert.alert(t('common.error'), t('app.failedDeleteConversation'));
    }
  }, [currentConversation?.id, invalidateConversationList]);

  const renameConversation = useCallback(async (conversationId: string, newTitle: string) => {
    try {
      setError(null);
      const updated = await chatApi.updateConversation(conversationId, { title: newTitle });
      setConversations(prev => {
        const updatedConv = prev.find(c => c.id === conversationId);
        if (!updatedConv) return prev;
        const others = prev.filter(c => c.id !== conversationId);
        return [
          { ...updatedConv, title: updated.title, updated_at: updated.updated_at },
          ...others,
        ];
      });
      setCurrentConversation(prev => {
        if (prev?.id === conversationId) {
          return { ...prev, title: updated.title, updated_at: updated.updated_at };
        }
        return prev;
      });
      invalidateConversationList();
    } catch (err) {
      const errorMessage =
        err instanceof Error ? err.message : t('app.failedRenameConversation');
      setError(errorMessage);
      console.error('Failed to rename conversation:', err);
      Alert.alert(t('common.error'), t('app.failedRenameConversation'));
    }
  }, [invalidateConversationList]);

  /**
   * Move the athlete onto `conversationId`, reading the row from the server.
   *
   * `/reset` forges its fresh thread server-side, so the only copy of it this
   * screen can have is the one the list read returns. Answers whether the row
   * was found: a switch that cannot resolve its thread leaves the athlete
   * where they are rather than on a blank screen.
   */
  const switchToConversation = useCallback(async (conversationId: string): Promise<boolean> => {
    try {
      const response = await chatApi.getConversations();
      const found = (response.conversations ?? []).find(c => c.id === conversationId);
      if (!found) return false;
      setConversations(prev => [found, ...prev.filter(c => c.id !== found.id)]);
      setCurrentConversation(found);
      invalidateConversationList();
      return true;
    } catch (err) {
      console.error('Failed to open the conversation the turn moved to:', err);
      return false;
    }
  }, [invalidateConversationList]);

  return {
    conversations,
    currentConversation,
    isLoading,
    error,
    loadConversations,
    setCurrentConversation,
    createConversation,
    switchToConversation,
    deleteConversation,
    renameConversation,
    justCreatedConversationRef,
  };
}
