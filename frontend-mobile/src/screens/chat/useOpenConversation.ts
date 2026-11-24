// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Which conversation the chat screen has open, resolved out of the one React Query conversation list
// ABOUTME: Holds only the open id; the row, its rename, its delete and its creation are the list's own

import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject } from 'react';
import { Alert } from 'react-native';
import type { CreateConversationOptions } from '@pierre/api-client';
import { useTranslation } from '@pierre/i18n';
import type { Conversation } from '../../types';
import { useConversationList } from '../conversations/useConversationList';
import { useCreateConversation } from './usePersonalThread';

/** The open conversation and what the chat screen can do to it. */
export interface OpenConversation {
  /** Every conversation the list has read, the same rows the list screen draws. */
  conversations: Conversation[];
  /** The id the screen asked for; `null` on the new thread. */
  openId: string | null;
  /**
   * The open conversation's row, read out of the list. `null` on the new
   * thread, and while the list has not yet read the row `openId` names.
   */
  currentConversation: Conversation | null;
  /** True until the list's first read answers. */
  isLoading: boolean;
  /** Read the list again, joining a read already on its way. */
  refresh: () => void;
  /** Open the conversation `id` names, or the new thread with `null`. */
  openConversation: (id: string | null) => void;
  /** Create a conversation, open it, and answer its id. */
  createConversation: (options: CreateConversationOptions) => Promise<string>;
  /**
   * Open `conversationId`, reading the list from the server first.
   *
   * For a thread the server forged rather than this screen: `/reset` archives
   * the current one and continues on a fresh row nothing local has yet.
   * Answers whether the row was found: a switch that cannot resolve its
   * thread leaves the athlete where they are rather than on a blank screen.
   */
  switchToConversation: (conversationId: string) => Promise<boolean>;
  /** Delete a conversation; a failure is told to the athlete, never thrown. */
  deleteConversation: (conversationId: string) => Promise<void>;
  /** Rename a conversation; a failure is told to the athlete, never thrown. */
  renameConversation: (conversationId: string, newTitle: string) => Promise<void>;
  /**
   * The id of a thread created for the line being sent, until its first read
   * is skipped. Such a thread is empty — its agent posts no welcome — and a
   * read still in flight when the first line is sent would land after it and
   * wipe it. Any other new thread carries its agent's welcome and is read.
   */
  justCreatedConversationRef: MutableRefObject<string | null>;
}

/**
 * The chat screen's open conversation.
 *
 * The conversation list is one React Query cache, read by the list screen,
 * the tab badge and this hook. What is kept here is the id of the open
 * thread alone: its row is looked up in the list, a rename or a delete goes
 * through the list's own mutations, and a new thread through the one create
 * path, which puts the created row in the list before this hook looks for it.
 */
export function useOpenConversation(): OpenConversation {
  const { t } = useTranslation();
  const list = useConversationList();
  const create = useCreateConversation();
  const [openId, setOpenId] = useState<string | null>(null);
  const justCreatedConversationRef = useRef<string | null>(null);

  const { conversations, refetch, rename, remove, hasMore, isLoading, isLoadingMore, loadMore } = list;

  const currentConversation = useMemo(
    () => (openId === null ? null : conversations.find((conversation) => conversation.id === openId) ?? null),
    [conversations, openId],
  );

  // The list is paged. A thread opened by id — a deep link, a notification —
  // may sit below the rows read so far, so the next page is asked for until
  // the row turns up or the list ends.
  useEffect(() => {
    if (openId === null || currentConversation !== null) return;
    if (isLoading || isLoadingMore || !hasMore) return;
    loadMore();
  }, [openId, currentConversation, isLoading, isLoadingMore, hasMore, loadMore]);

  const refresh = useCallback(() => {
    void refetch({ cancelRefetch: false });
  }, [refetch]);

  const createConversation = useCallback(
    async (options: CreateConversationOptions): Promise<string> => {
      const id = await create(options);
      if (!id) {
        throw new Error(t('app.invalidConversationResponse'));
      }
      justCreatedConversationRef.current = options.athlete_speaks_first ? id : null;
      setOpenId(id);
      return id;
    },
    [create, t],
  );

  const switchToConversation = useCallback(
    async (conversationId: string): Promise<boolean> => {
      const read = await refetch();
      if (read.isError) {
        console.error('Failed to open the conversation the turn moved to:', read.error);
        return false;
      }
      const found = (read.data?.pages ?? []).some((page) =>
        (page.conversations ?? []).some((conversation) => conversation.id === conversationId),
      );
      if (!found) return false;
      setOpenId(conversationId);
      return true;
    },
    [refetch],
  );

  const deleteConversation = useCallback(
    async (conversationId: string): Promise<void> => {
      try {
        await remove(conversationId);
        setOpenId((current) => (current === conversationId ? null : current));
      } catch (err) {
        console.error('Failed to delete conversation:', err);
        Alert.alert(t('common.error'), t('app.failedDeleteConversation'));
      }
    },
    [remove, t],
  );

  const renameConversation = useCallback(
    async (conversationId: string, newTitle: string): Promise<void> => {
      try {
        await rename(conversationId, newTitle);
      } catch (err) {
        console.error('Failed to rename conversation:', err);
        Alert.alert(t('common.error'), t('app.failedRenameConversation'));
      }
    },
    [rename, t],
  );

  return {
    conversations,
    openId,
    currentConversation,
    isLoading,
    refresh,
    openConversation: setOpenId,
    createConversation,
    switchToConversation,
    deleteConversation,
    renameConversation,
    justCreatedConversationRef,
  };
}
