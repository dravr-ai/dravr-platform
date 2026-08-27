// ABOUTME: Hook for managing chat conversations CRUD operations and state
// ABOUTME: Handles loading, creating, updating, deleting, and selecting conversations

import React, { useState, useCallback, useRef } from 'react';
import { Alert } from 'react-native';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { chatApi } from '../../services/api';
import { extractErrorMessage } from '../../utils/errorMessages';
import type { Conversation } from '../../types';

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
  deleteConversation: (conversationId: string) => Promise<void>;
  renameConversation: (conversationId: string, newTitle: string) => Promise<void>;
  handleNewChat: () => void;
  updateConversationInList: (conversation: Conversation) => void;
  addConversationToTop: (conversation: Conversation) => void;
  justCreatedConversationRef: React.MutableRefObject<string | null>;
}

export function useConversations(): ConversationsState & ConversationsActions {
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
      const errorMessage = err instanceof Error ? err.message : 'Failed to load conversations';
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
        throw new Error('Invalid conversation response');
      }
      setConversations(prev => [conversation, ...prev]);
      justCreatedConversationRef.current = conversation.id;
      setCurrentConversation(conversation);
      invalidateConversationList();
      return conversation;
    } catch (err) {
      const errorMessage = extractErrorMessage(err, 'Failed to create conversation');
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
      const errorMessage = err instanceof Error ? err.message : 'Failed to delete conversation';
      setError(errorMessage);
      Alert.alert('Error', 'Failed to delete conversation');
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
      const errorMessage = err instanceof Error ? err.message : 'Failed to rename conversation';
      setError(errorMessage);
      console.error('Failed to rename conversation:', err);
      Alert.alert('Error', 'Failed to rename conversation');
    }
  }, [invalidateConversationList]);

  const handleNewChat = useCallback(() => {
    setCurrentConversation(null);
  }, []);

  const updateConversationInList = useCallback((conversation: Conversation) => {
    setConversations(prev => {
      const others = prev.filter(c => c.id !== conversation.id);
      return [conversation, ...others];
    });
  }, []);

  const addConversationToTop = useCallback((conversation: Conversation) => {
    setConversations(prev => [conversation, ...prev]);
    justCreatedConversationRef.current = conversation.id;
    setCurrentConversation(conversation);
  }, []);

  return {
    conversations,
    currentConversation,
    isLoading,
    error,
    loadConversations,
    setCurrentConversation,
    createConversation,
    deleteConversation,
    renameConversation,
    handleNewChat,
    updateConversationInList,
    addConversationToTop,
    justCreatedConversationRef,
  };
}
