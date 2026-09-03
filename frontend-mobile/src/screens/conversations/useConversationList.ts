// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The unified conversation list on React Query — rows, paging, unread total, rename/delete/read mutations
// ABOUTME: One query key feeds the list screen and the tab badge, so both read the same rows

import { useCallback, useMemo } from 'react';
import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
} from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import {
  buildConversationRow,
  conversationRowLabels,
  sortRowsByActivity,
  type ConversationRowModel,
} from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import type { ConversationsResponse } from '@pierre/api-client';
import { chatApi } from '../../services/api';
import type { Conversation } from '../../types';

/** How many rows one page of the list asks the server for. */
export const CONVERSATION_PAGE_SIZE = 50;

type ConversationPages = InfiniteData<ConversationsResponse, number>;

/**
 * Rewrite one conversation in every cached page, or drop it when `patch`
 * returns null. A list that has not loaded is left alone — there is nothing
 * to patch, and the next read fetches the truth anyway.
 */
export function patchCachedConversation(
  queryClient: QueryClient,
  conversationId: string,
  patch: (conversation: Conversation) => Conversation | null,
): void {
  queryClient.setQueryData<ConversationPages>(QUERY_KEYS.chat.conversations(), (data) => {
    if (!data) return data;
    return {
      ...data,
      pages: data.pages.map((page) => ({
        ...page,
        conversations: page.conversations.flatMap((conversation) => {
          if (conversation.id !== conversationId) return [conversation];
          const next = patch(conversation);
          return next ? [next] : [];
        }),
      })),
    };
  });
}

function loadedCount(pages: readonly ConversationsResponse[]): number {
  return pages.reduce((count, page) => count + (page.conversations?.length ?? 0), 0);
}

/**
 * The conversations the caller takes part in, as the server lists them.
 *
 * Paged on `offset` against the server's real `total`; the next page is
 * asked for only while the last one was non-empty and fewer rows are loaded
 * than the total says exist. Rows are de-duplicated by id across pages so a
 * conversation that moved between two fetches is never drawn twice.
 */
export function useConversationRows() {
  const query = useInfiniteQuery({
    queryKey: QUERY_KEYS.chat.conversations(),
    queryFn: ({ pageParam }) => chatApi.getConversations(CONVERSATION_PAGE_SIZE, pageParam),
    initialPageParam: 0,
    getNextPageParam: (lastPage, pages) => {
      const loaded = loadedCount(pages);
      const total = typeof lastPage.total === 'number' ? lastPage.total : loaded;
      return (lastPage.conversations?.length ?? 0) > 0 && loaded < total ? loaded : undefined;
    },
    // The list refetches on focus and after every mutation that moves a row;
    // between those, thirty seconds of staleness is invisible to the athlete.
    staleTime: 30_000,
  });

  const conversations = useMemo<Conversation[]>(() => {
    const seen = new Set<string>();
    const unique: Conversation[] = [];
    for (const page of query.data?.pages ?? []) {
      for (const conversation of page.conversations ?? []) {
        if (seen.has(conversation.id)) continue;
        seen.add(conversation.id);
        unique.push(conversation);
      }
    }
    return unique;
  }, [query.data]);

  // The words the shared row model cannot spell itself, in the viewer's
  // language — the same key set the web list resolves.
  const { t, language } = useTranslation();
  const labels = useMemo(() => conversationRowLabels(t, language), [t, language]);

  const rows = useMemo<ConversationRowModel[]>(
    () =>
      sortRowsByActivity(
        conversations.map((conversation) => buildConversationRow(conversation, labels)),
      ),
    [conversations, labels],
  );

  const unreadTotal = useMemo(
    () => conversations.reduce((sum, conversation) => sum + Math.max(0, conversation.unread_count ?? 0), 0),
    [conversations],
  );

  const pages = query.data?.pages ?? [];
  const lastPage = pages[pages.length - 1];
  const total = typeof lastPage?.total === 'number' ? lastPage.total : conversations.length;

  const loadMore = useCallback(() => {
    if (query.hasNextPage && !query.isFetchingNextPage) {
      void query.fetchNextPage();
    }
  }, [query]);

  return {
    rows,
    conversations,
    total,
    unreadTotal,
    isLoading: query.isLoading,
    isRefetching: query.isRefetching && !query.isFetchingNextPage,
    isError: query.isError,
    error: query.error,
    refetch: query.refetch,
    hasMore: query.hasNextPage,
    isLoadingMore: query.isFetchingNextPage,
    loadMore,
  };
}

/**
 * The list plus the four things a row can do: rename, delete, mark read,
 * mark unread. Each patches the cache first so the row reflects the action
 * before the server confirms, then invalidates so the truth is re-read.
 */
export function useConversationList() {
  const queryClient = useQueryClient();
  const list = useConversationRows();

  const invalidate = useCallback(
    () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() }),
    [queryClient],
  );

  const rename = useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) => chatApi.updateConversation(id, { title }),
    onSuccess: (updated, { id }) => {
      patchCachedConversation(queryClient, id, (conversation) => ({
        ...conversation,
        title: updated.title,
        updated_at: updated.updated_at,
      }));
    },
    onSettled: invalidate,
  });

  const remove = useMutation({
    mutationFn: (id: string) => chatApi.deleteConversation(id),
    onMutate: (id) => {
      patchCachedConversation(queryClient, id, () => null);
    },
    onSettled: invalidate,
  });

  const markRead = useMutation({
    mutationFn: (id: string) => chatApi.markConversationRead(id),
    onMutate: (id) => {
      patchCachedConversation(queryClient, id, (conversation) => ({ ...conversation, unread_count: 0 }));
    },
    onSettled: invalidate,
  });

  const markUnread = useMutation({
    mutationFn: (id: string) => chatApi.markConversationUnread(id),
    onMutate: (id) => {
      // Clearing the marker makes every user/assistant row unread again —
      // the count `message_count` already carries.
      patchCachedConversation(queryClient, id, (conversation) => ({
        ...conversation,
        unread_count: conversation.message_count,
      }));
    },
    onSettled: invalidate,
  });

  return {
    ...list,
    rename: (id: string, title: string) => rename.mutateAsync({ id, title }),
    remove: (id: string) => remove.mutateAsync(id),
    markRead: (id: string) => markRead.mutateAsync(id),
    markUnread: (id: string) => markUnread.mutateAsync(id),
  };
}
