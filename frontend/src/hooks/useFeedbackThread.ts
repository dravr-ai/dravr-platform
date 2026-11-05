// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's feedback thread with the Dravr team — read through React Query, a sent message appended in place
// ABOUTME: The team replies from Slack, so an open thread re-reads now and then and marks those replies read (carnet#1392)

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { FeedbackMessage, FeedbackThreadResponse, SendFeedbackRequest } from '@pierre/shared-types';
import { feedbackApi } from '../services/api';

/** How often an open thread looks for a reply from the team. */
const THREAD_REFRESH_MS = 60_000;

/** How often the unread dot looks for one: the notification bell's cadence. */
const UNREAD_REFRESH_MS = 120_000;

/**
 * The thread and the one write it takes. A sent message joins the cached
 * thread as the server answered it, so the thread never re-reads to show
 * what was just written. Showing a team reply reads it: once the thread has
 * loaded with team replies in it, they are marked read and the unread dot
 * clears.
 */
export function useFeedbackThread() {
  const queryClient = useQueryClient();
  const thread = useQuery({
    queryKey: QUERY_KEYS.user.feedbackThread(),
    queryFn: () => feedbackApi.getThread(),
    refetchInterval: THREAD_REFRESH_MS,
  });
  const send = useMutation({
    mutationFn: (request: SendFeedbackRequest) => feedbackApi.sendMessage(request),
    onSuccess: (message: FeedbackMessage) => {
      queryClient.setQueryData<FeedbackThreadResponse>(QUERY_KEYS.user.feedbackThread(), (current) => ({
        messages: [...(current?.messages ?? []), message],
      }));
    },
  });
  const { mutate: markRead } = useMutation({
    mutationFn: () => feedbackApi.markRead(),
    onSuccess: () => {
      queryClient.setQueryData<number>(QUERY_KEYS.user.feedbackUnread(), 0);
    },
  });

  const teamReplies = thread.data?.messages.filter((message) => message.author === 'team').length ?? 0;
  useEffect(() => {
    if (teamReplies > 0) markRead();
  }, [teamReplies, markRead]);

  return { thread, send };
}

/** A screenshot on the screen: its local URL once read, or that it could not be read. */
export interface FeedbackImageView {
  url: string | null;
  failed: boolean;
}

/**
 * A local URL for the screenshot one of the athlete's messages carries, read
 * once through the API client (so it is fetched as the athlete, whatever the
 * API's origin) and let go of when the message leaves the screen.
 */
export function useFeedbackImageUrl(messageId: string): FeedbackImageView {
  const image = useQuery({
    queryKey: QUERY_KEYS.user.feedbackImage(messageId),
    queryFn: () => feedbackApi.getImage(messageId),
    staleTime: Infinity,
  });
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!image.data) return undefined;
    const objectUrl = URL.createObjectURL(image.data);
    setUrl(objectUrl);
    return () => URL.revokeObjectURL(objectUrl);
  }, [image.data]);
  return { url, failed: image.isError };
}

/** How many of the team's replies are waiting unread: the account menu's dot. */
export function useFeedbackUnread(): number {
  const unread = useQuery({
    queryKey: QUERY_KEYS.user.feedbackUnread(),
    queryFn: () => feedbackApi.getUnread(),
    refetchInterval: UNREAD_REFRESH_MS,
  });
  return unread.data ?? 0;
}
