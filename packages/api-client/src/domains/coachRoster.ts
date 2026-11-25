// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach roster domain API - the human coach's athletes for Coach Home, "Traité" on an attention reason, and the morning brief
// ABOUTME: One read feeds À traiter, Tableau d'équipe and La saison; dismiss and restore write the coach's handled marks; the brief is read, changed and told of an open

import type { AxiosInstance } from 'axios';
import type {
  CoachRosterResponse,
  CoachRosterQuery,
  CoachAttentionDismissRequest,
  CoachAttentionDismissResponse,
  CoachAttentionRestoreRequest,
  CoachAttentionRestoreResponse,
  CoachBrief,
  CoachBriefUpdate,
  CoachBriefOpenedRequest,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type {
  CoachRosterResponse,
  CoachRosterQuery,
  CoachAttentionDismissRequest,
  CoachAttentionDismissResponse,
  CoachAttentionRestoreRequest,
  CoachAttentionRestoreResponse,
  CoachBrief,
  CoachBriefUpdate,
  CoachBriefOpenedRequest,
};

/**
 * Creates the coach roster API methods bound to an axios instance.
 */
export function createCoachRosterApi(axios: AxiosInstance) {
  return {
    /**
     * One page of the coach's roster (`limit` 1–100, 50 when absent). Each
     * row carries its ranked attention reasons; a roster longer than one page
     * is read by following `next_offset`, and reasons merge across pages on
     * their `rank`.
     */
    async getRoster(query?: CoachRosterQuery): Promise<CoachRosterResponse> {
      const response = await axios.get<CoachRosterResponse>(ENDPOINTS.COACH_ROSTER.ROSTER, {
        params: query,
      });
      return response.data;
    },

    /** Mark one attention reason handled for the signal it was raised on. */
    async dismissAttention(
      request: CoachAttentionDismissRequest
    ): Promise<CoachAttentionDismissResponse> {
      const response = await axios.post<CoachAttentionDismissResponse>(
        ENDPOINTS.COACH_ROSTER.ATTENTION_DISMISS,
        request
      );
      return response.data;
    },

    /** Undo a dismissal. */
    async restoreAttention(
      request: CoachAttentionRestoreRequest
    ): Promise<CoachAttentionRestoreResponse> {
      const response = await axios.post<CoachAttentionRestoreResponse>(
        ENDPOINTS.COACH_ROSTER.ATTENTION_RESTORE,
        request
      );
      return response.data;
    },

    /** The coach's morning brief: its settings and its last run. 403 for a caller who coaches no one. */
    async getBrief(): Promise<CoachBrief> {
      const response = await axios.get<CoachBrief>(ENDPOINTS.COACH_ROSTER.BRIEF);
      return response.data;
    },

    /** Change the brief: only the fields given move, and a list given replaces the stored one in full. */
    async updateBrief(update: CoachBriefUpdate): Promise<CoachBrief> {
      const response = await axios.put<CoachBrief>(ENDPOINTS.COACH_ROSTER.BRIEF, update);
      return response.data;
    },

    /** Tell the server the coach opened this athlete's detail (204). */
    async markBriefOpened(athleteUserId: string): Promise<void> {
      const body: CoachBriefOpenedRequest = { athlete_user_id: athleteUserId };
      await axios.post(ENDPOINTS.COACH_ROSTER.BRIEF_OPENED, body);
    },
  };
}

export type CoachRosterApi = ReturnType<typeof createCoachRosterApi>;
