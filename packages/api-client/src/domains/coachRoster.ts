// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach roster domain API - the human coach's athletes for Coach Home, and "Traité" on an attention reason
// ABOUTME: One read feeds À traiter, Tableau d'équipe and La saison; dismiss and restore write the coach's handled marks

import type { AxiosInstance } from 'axios';
import type {
  CoachRosterResponse,
  CoachRosterQuery,
  CoachAttentionDismissRequest,
  CoachAttentionDismissResponse,
  CoachAttentionRestoreRequest,
  CoachAttentionRestoreResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type {
  CoachRosterResponse,
  CoachRosterQuery,
  CoachAttentionDismissRequest,
  CoachAttentionDismissResponse,
  CoachAttentionRestoreRequest,
  CoachAttentionRestoreResponse,
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
  };
}

export type CoachRosterApi = ReturnType<typeof createCoachRosterApi>;
