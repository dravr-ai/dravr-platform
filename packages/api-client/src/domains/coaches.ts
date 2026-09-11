// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coaches domain API - list, read, update and delete the caller's coaches
// ABOUTME: Creation, install and catalogue browsing live in /coach and /discover

import type { AxiosInstance } from 'axios';
import type {
  Agent,
  UpdateAgentRequest,
  ListAgentsResponse,
  AgentProposalResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type { Agent, UpdateAgentRequest, ListAgentsResponse, AgentProposalResponse };

export interface ListAgentsOptions {
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
    async list(options?: ListAgentsOptions): Promise<ListAgentsResponse> {
      const params = new URLSearchParams();
      if (options?.category) params.append('category', options.category);
      if (options?.favorites_only) params.append('favorites_only', 'true');
      if (options?.include_hidden) params.append('include_hidden', 'true');
      if (options?.limit) params.append('limit', options.limit.toString());
      if (options?.offset) params.append('offset', options.offset.toString());
      if (options?.personalize) params.append('personalize', 'true');

      const queryString = params.toString();
      const url = queryString ? `${ENDPOINTS.COACHES.LIST}?${queryString}` : ENDPOINTS.COACHES.LIST;

      const response = await axios.get<ListAgentsResponse>(url);
      return response.data;
    },

    /**
     * Onboarding coach proposal: returns the user's inferred sport profile plus
     * the top (≤3) coaches for them, each with a one-line rationale. Backs the
     * post-onboarding "we analyzed your data → here are your coaches" screen.
     */
    async getProposal(): Promise<AgentProposalResponse> {
      const response = await axios.get<AgentProposalResponse>(ENDPOINTS.COACHES.PROPOSAL);
      return response.data;
    },

    /**
     * Get a specific coach by ID.
     */
    async get(agentId: string): Promise<Agent> {
      const response = await axios.get<Agent>(ENDPOINTS.COACHES.COACH(agentId));
      return response.data;
    },

    /**
     * Update an existing coach.
     */
    async update(agentId: string, request: UpdateAgentRequest): Promise<Agent> {
      const response = await axios.put<Agent>(ENDPOINTS.COACHES.COACH(agentId), request);
      return response.data;
    },

    /**
     * Delete a coach.
     */
    async delete(agentId: string): Promise<void> {
      await axios.delete(ENDPOINTS.COACHES.COACH(agentId));
    },

    /**
     * Record coach usage (for analytics).
     */
    async recordUsage(agentId: string): Promise<void> {
      try {
        await axios.post(ENDPOINTS.COACHES.USAGE(agentId));
      } catch {
        // Silent failure - usage tracking is non-critical
      }
    },
  };
}

export type AgentsApi = ReturnType<typeof createCoachesApi>;
