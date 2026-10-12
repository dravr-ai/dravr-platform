// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Web client for a coach's review of the agent's group replies — the page of replies, a flag, a correction, the weekly count
// ABOUTME: The coach's review page is a web surface, so the client sits with the web-only modules; its wire types are shared (carnet#847)

import type {
  CoachRepliesPage,
  CoachRepliesQuery,
  CoachReviewWeeklyResponse,
  CorrectReplyResponse,
  FlagReplyResponse,
  ReplyFlagReason,
} from '@pierre/shared-types';
import { axios } from './client';

export const coachRepliesApi = {
  /**
   * One page of the agent's replies in the groups the caller coaches, newest
   * first. `before` is the `next_before` of the page already held.
   */
  async list(query: CoachRepliesQuery = {}): Promise<CoachRepliesPage> {
    const params: CoachRepliesQuery = {};
    if (query.athlete_id) params.athlete_id = query.athlete_id;
    if (query.group_id) params.group_id = query.group_id;
    if (query.before) params.before = query.before;
    if (query.limit !== undefined) params.limit = query.limit;
    const response = await axios.get<CoachRepliesPage>('/api/coach/replies', { params });
    return response.data;
  },

  /** Flag a reply. The athlete never sees it. */
  async flag(replyId: string, reason: ReplyFlagReason, note?: string): Promise<FlagReplyResponse> {
    const response = await axios.post<FlagReplyResponse>(
      `/api/coach/replies/${encodeURIComponent(replyId)}/flags`,
      { reason, note: note?.trim() ? note.trim() : undefined },
    );
    return response.data;
  },

  /** Post a correction into the group thread, linked to the reply it corrects. */
  async correct(replyId: string, content: string): Promise<CorrectReplyResponse> {
    const response = await axios.post<CorrectReplyResponse>(
      `/api/coach/replies/${encodeURIComponent(replyId)}/corrections`,
      { content },
    );
    return response.data;
  },

  /** The caller's flags and corrections per week, oldest week first. */
  async weekly(weeks?: number): Promise<CoachReviewWeeklyResponse> {
    const response = await axios.get<CoachReviewWeeklyResponse>('/api/coach/replies/weekly', {
      params: weeks === undefined ? undefined : { weeks },
    });
    return response.data;
  },
};
