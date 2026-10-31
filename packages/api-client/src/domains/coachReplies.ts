// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach replies domain API - the human coach's review of the agent's group replies: the page of replies, a flag, a correction, the weekly count
// ABOUTME: Shared by the web Replies page and the phone's reply review; wire types come from @pierre/shared-types (carnet#847, carnet#1304)

import type { AxiosInstance } from 'axios';
import type {
  CoachRepliesPage,
  CoachRepliesQuery,
  CoachReviewWeeklyResponse,
  CorrectReplyResponse,
  FlagReplyResponse,
  ReplyFlagReason,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type {
  CoachRepliesPage,
  CoachRepliesQuery,
  CoachReviewWeeklyResponse,
  CorrectReplyResponse,
  FlagReplyResponse,
  ReplyFlagReason,
};

/**
 * Creates the coach replies API methods bound to an axios instance.
 */
export function createCoachRepliesApi(axios: AxiosInstance) {
  return {
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
      const response = await axios.get<CoachRepliesPage>(ENDPOINTS.COACH_REPLIES.LIST, { params });
      return response.data;
    },

    /** Flag a reply. The athlete never sees it. */
    async flag(replyId: string, reason: ReplyFlagReason, note?: string): Promise<FlagReplyResponse> {
      const trimmed = note?.trim();
      const response = await axios.post<FlagReplyResponse>(ENDPOINTS.COACH_REPLIES.FLAGS(replyId), {
        reason,
        note: trimmed ? trimmed : undefined,
      });
      return response.data;
    },

    /** Post a correction into the group thread, linked to the reply it corrects. */
    async correct(replyId: string, content: string): Promise<CorrectReplyResponse> {
      const response = await axios.post<CorrectReplyResponse>(ENDPOINTS.COACH_REPLIES.CORRECTIONS(replyId), {
        content,
      });
      return response.data;
    },

    /** The caller's flags and corrections per week, oldest week first. */
    async weekly(weeks?: number): Promise<CoachReviewWeeklyResponse> {
      const response = await axios.get<CoachReviewWeeklyResponse>(ENDPOINTS.COACH_REPLIES.WEEKLY, {
        params: weeks === undefined ? undefined : { weeks },
      });
      return response.data;
    },
  };
}

export type CoachRepliesApi = ReturnType<typeof createCoachRepliesApi>;
