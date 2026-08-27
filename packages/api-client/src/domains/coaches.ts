// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coaches domain API - list, read, update and delete the caller's coaches
// ABOUTME: Creation, install and catalogue browsing live in /coach and /discover

import type { AxiosInstance } from 'axios';
import type {
  Coach,
  UpdateCoachRequest,
  ListCoachesResponse,
  CoachProposalResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type { Coach, UpdateCoachRequest, ListCoachesResponse, CoachProposalResponse };

export interface ListCoachesOptions {
  category?: string;
  favorites_only?: boolean;
  include_hidden?: boolean;
  limit?: number;
  offset?: number;
  /** Mark each coach with a match_score + recommended flag based on the
   *  user's recent activities and connected providers. */
  personalize?: boolean;
}

/**
 * Creates the coaches API methods bound to an axios instance.
 */
export function createCoachesApi(axios: AxiosInstance) {
  return {
    /**
     * List coaches with optional filters.
     */
    async list(options?: ListCoachesOptions): Promise<ListCoachesResponse> {
      const params = new URLSearchParams();
      if (options?.category) params.append('category', options.category);
      if (options?.favorites_only) params.append('favorites_only', 'true');
      if (options?.include_hidden) params.append('include_hidden', 'true');
      if (options?.limit) params.append('limit', options.limit.toString());
      if (options?.offset) params.append('offset', options.offset.toString());
      if (options?.personalize) params.append('personalize', 'true');

      const queryString = params.toString();
      const url = queryString ? `${ENDPOINTS.COACHES.LIST}?${queryString}` : ENDPOINTS.COACHES.LIST;

      const response = await axios.get<ListCoachesResponse>(url);
      return response.data;
    },

    /**
     * Onboarding coach proposal: returns the user's inferred sport profile plus
     * the top (≤3) coaches for them, each with a one-line rationale. Backs the
     * post-onboarding "we analyzed your data → here are your coaches" screen.
     */
    async getProposal(): Promise<CoachProposalResponse> {
      const response = await axios.get<CoachProposalResponse>(ENDPOINTS.COACHES.PROPOSAL);
      return response.data;
    },

    /**
     * Get a specific coach by ID.
     */
    async get(coachId: string): Promise<Coach> {
      const response = await axios.get<Coach>(ENDPOINTS.COACHES.COACH(coachId));
      return response.data;
    },

    /**
     * Update an existing coach.
     */
    async update(coachId: string, request: UpdateCoachRequest): Promise<Coach> {
      const response = await axios.put<Coach>(ENDPOINTS.COACHES.COACH(coachId), request);
      return response.data;
    },

    /**
     * Delete a coach.
     */
    async delete(coachId: string): Promise<void> {
      await axios.delete(ENDPOINTS.COACHES.COACH(coachId));
    },

    /**
     * Record coach usage (for analytics).
     */
    async recordUsage(coachId: string): Promise<void> {
      try {
        await axios.post(ENDPOINTS.COACHES.USAGE(coachId));
      } catch {
        // Silent failure - usage tracking is non-critical
      }
    },
  };
}

export type CoachesApi = ReturnType<typeof createCoachesApi>;
