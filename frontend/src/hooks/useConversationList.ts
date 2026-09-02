// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one query behind the unified conversation list — paged rows, search, and the row mutations
// ABOUTME: Every web reader of GET /api/chat/conversations goes through here so the cache has one shape

import { useCallback, useMemo } from 'react';
import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
} from '@tanstack/react-query';
import {
  buildConversationRow,
  CONVERSATION_ROW_LABEL_KEYS,
  filterRows,
  sortRowsByActivity,
  type ConversationRowLabels,
  type ConversationRowModel,
} from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import type { ConversationsResponse } from '@pierre/api-client';
import type { Conversation } from '@pierre/shared-types';
import { chatApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

/** Rows per page — the server clamps the list to `1..=200`, and 50 fills a sidebar many times over. */
export const CONVERSATION_PAGE_SIZE = 50;

/** The cache shape under `QUERY_KEYS.chat.conversations()`: one page per fetch, offset-keyed. */
export type ConversationPages = InfiniteData<ConversationsResponse, number>;

function loadedCount(pages: readonly ConversationsResponse[]): number {
  return pages.reduce((count, page) => count + page.conversations.length, 0);
}

/**
 * The paged list query itself, shared by everything that reads the list —
 * the sidebar, the open thread's header, the nav badge. One observer per
 * caller, one cache entry, so a rename in the sidebar is the header's rename.
 *
 * `enabled` lets a caller that only wants a count (the nav badge) stay quiet
 * for a role that has no chat tab.
 */
export function useConversationsQuery(enabled = true) {
  return useInfiniteQuery({
    queryKey: QUERY_KEYS.chat.conversations(),
    queryFn: ({ pageParam }) => chatApi.getConversations(CONVERSATION_PAGE_SIZE, pageParam),
    initialPageParam: 0,
    // The server reports the real participant total; the next offset is what
    // is loaded so far, and there is one only while the total is not reached.
    getNextPageParam: (lastPage, allPages) => {
      const loaded = loadedCount(allPages);
      return lastPage.conversations.length > 0 && loaded < lastPage.total ? loaded : undefined;
    },
    enabled,
  });
}

/** Every conversation the cache holds, across pages, in server order. */
export function cachedConversations(queryClient: QueryClient): Conversation[] {
  const data = queryClient.getQueryData<ConversationPages>(QUERY_KEYS.chat.conversations());
  return data?.pages.flatMap((page) => page.conversations) ?? [];
}

/**
 * Rewrite one cached row in place — the optimistic half of a mutation whose
 * server answer is a refetch. A row the cache does not hold is left alone.
 */
export function patchCachedConversation(
  queryClient: QueryClient,
  conversationId: string,
  patch: Partial<Conversation>,
): void {
  queryClient.setQueryData<ConversationPages>(QUERY_KEYS.chat.conversations(), (old) => {
    if (!old) return old;
    return {
      ...old,
      pages: old.pages.map((page) => ({
        ...page,
        conversations: page.conversations.map((conversation) =>
          conversation.id === conversationId ? { ...conversation, ...patch } : conversation,
        ),
      })),
    };
  });
}

/** What the list and the thread header read. */
export interface ConversationListState {
  /** The rows to draw, newest activity first, narrowed by `query`. */
  rows: ConversationRowModel[];
  /** The raw rows the server sent, across every loaded page. */
  conversations: Conversation[];
  /** How many conversations the caller takes part in, loaded or not. */
  total: number;
  isLoading: boolean;
  isError: boolean;
  error: Error | null;
  /** True while the server holds rows beyond the loaded pages. */
  hasMore: boolean;
  isLoadingMore: boolean;
  loadMore: () => void;
  refetch: () => void;
}

/**
 * The unified list: every conversation the athlete takes part in, whatever
 * created it, as one flat set of rows sorted by last activity.
 *
 * `query` narrows the rows the way the search box does — title, coach handle
 * or preview — without touching what is cached, so clearing the box costs no
 * request.
 */
export function useConversationList(query = ''): ConversationListState {
  const result = useConversationsQuery();

  const conversations = useMemo(
    () => result.data?.pages.flatMap((page) => page.conversations) ?? [],
    [result.data],
  );

  const total = useMemo(() => {
    const pages = result.data?.pages ?? [];
    const last = pages[pages.length - 1];
    return last?.total ?? conversations.length;
  }, [result.data, conversations.length]);

  // The words the shared row model cannot spell itself, in the viewer's
  // language — so the same key set feeds a row here and on mobile.
  const { t, language } = useTranslation();
  const labels = useMemo<ConversationRowLabels>(
    () => ({
      locale: language,
      you: t(CONVERSATION_ROW_LABEL_KEYS.you),
      coach: t(CONVERSATION_ROW_LABEL_KEYS.coach),
      untitled: t(CONVERSATION_ROW_LABEL_KEYS.untitled),
    }),
    [t, language],
  );

  const rows = useMemo(() => {
    const now = new Date();
    const built = sortRowsByActivity(
      conversations.map((conversation) => buildConversationRow(conversation, labels, now)),
    );
    return filterRows(built, query);
  }, [conversations, labels, query]);

  const { fetchNextPage, refetch } = result;
  const loadMore = useCallback(() => {
    void fetchNextPage();
  }, [fetchNextPage]);
  const refetchList = useCallback(() => {
    void refetch();
  }, [refetch]);

  return {
    rows,
    conversations,
    total,
    isLoading: result.isLoading,
    isError: result.isError,
    error: result.error,
    hasMore: result.hasNextPage,
    isLoadingMore: result.isFetchingNextPage,
    loadMore,
    refetch: refetchList,
  };
}

/**
 * `user`/`assistant` rows the athlete has not read, summed over every loaded
 * conversation — what the Chat nav badge shows.
 */
export function useUnreadConversationTotal(enabled = true): number {
  const { data } = useConversationsQuery(enabled);
  return useMemo(
    () =>
      data?.pages
        .flatMap((page) => page.conversations)
        .reduce((sum, conversation) => sum + Math.max(0, conversation.unread_count ?? 0), 0) ??
      0,
    [data],
  );
}

/** The row actions every list host offers: rename, delete, mark unread. */
export interface ConversationMutations {
  rename: (conversationId: string, title: string) => Promise<Conversation>;
  isRenaming: boolean;
  remove: (conversationId: string) => Promise<void>;
  isRemoving: boolean;
  markUnread: (conversationId: string) => Promise<void>;
  isMarkingUnread: boolean;
}

/**
 * The mutations behind a row's hover actions and the info panel's controls.
 *
 * Each one settles by refetching the list: the server derives the row —
 * title, `updated_at`, `unread_count` — and a refetch is what keeps the two
 * hosts that call these from drifting apart.
 */
export function useConversationMutations(): ConversationMutations {
  const queryClient = useQueryClient();
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });

  const renameMutation = useMutation({
    mutationFn: ({ conversationId, title }: { conversationId: string; title: string }) =>
      chatApi.updateConversation(conversationId, { title }),
    onSuccess: invalidate,
  });

  const removeMutation = useMutation({
    mutationFn: (conversationId: string) => chatApi.deleteConversation(conversationId),
    onSuccess: invalidate,
  });

  // Clearing the marker makes every row of the thread unread again; the
  // count the row then shows is the server's, so the list is refetched.
  const markUnreadMutation = useMutation({
    mutationFn: (conversationId: string) => chatApi.markConversationUnread(conversationId),
    onSuccess: invalidate,
  });

  const { mutateAsync: renameAsync } = renameMutation;
  const { mutateAsync: removeAsync } = removeMutation;
  const { mutateAsync: markUnreadAsync } = markUnreadMutation;

  const rename = useCallback(
    (conversationId: string, title: string) => renameAsync({ conversationId, title }),
    [renameAsync],
  );
  const remove = useCallback((conversationId: string) => removeAsync(conversationId), [removeAsync]);
  const markUnread = useCallback(
    (conversationId: string) => markUnreadAsync(conversationId),
    [markUnreadAsync],
  );

  return {
    rename,
    isRenaming: renameMutation.isPending,
    remove,
    isRemoving: removeMutation.isPending,
    markUnread,
    isMarkingUnread: markUnreadMutation.isPending,
  };
}
