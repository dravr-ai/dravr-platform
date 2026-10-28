// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's feedback thread with the Dravr team — read through React Query, a sent message appended in place
// ABOUTME: Nothing answers the thread automatically, so there is nothing to poll for (carnet#1235)

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { FeedbackMessage, FeedbackThreadResponse, SendFeedbackRequest } from '@pierre/shared-types';
import { feedbackApi } from '../services/api';

/**
 * The thread and the one write it takes. A sent message joins the cached
 * thread as the server answered it, so the thread never re-reads to show
 * what was just written.
 */
export function useFeedbackThread() {
  const queryClient = useQueryClient();
  const thread = useQuery({
    queryKey: QUERY_KEYS.user.feedbackThread(),
    queryFn: () => feedbackApi.getThread(),
  });
  const send = useMutation({
    mutationFn: (request: SendFeedbackRequest) => feedbackApi.sendMessage(request),
    onSuccess: (message: FeedbackMessage) => {
      queryClient.setQueryData<FeedbackThreadResponse>(QUERY_KEYS.user.feedbackThread(), (current) => ({
        messages: [...(current?.messages ?? []), message],
      }));
    },
  });
  return { thread, send };
}
