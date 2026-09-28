// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coaches domain API - list, read, update, delete and submit to the Store the caller's coaches, and their version history
// ABOUTME: Creation, install and catalogue browsing live in /coach and /discover

import type { AxiosInstance } from 'axios';
import type {
  Agent,
  UpdateAgentRequest,
  ListAgentsResponse,
  AgentProposalResponse,
  ListAgentVersionsResponse,
  AgentVersionDiffResponse,
  RevertAgentVersionResponse,
  SubmitAgentForReviewResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type {
  Agent,
  UpdateAgentRequest,
  ListAgentsResponse,
  AgentProposalResponse,
  ListAgentVersionsResponse,
  AgentVersionDiffResponse,
  RevertAgentVersionResponse,
  SubmitAgentForReviewResponse,
};

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
     * A coach's version history, newest first. Every edit snapshots the
     * content it replaces, so the current content is not among the versions.
     */
    async listVersions(agentId: string): Promise<ListAgentVersionsResponse> {
      const response = await axios.get<ListAgentVersionsResponse>(ENDPOINTS.COACHES.VERSIONS(agentId));
      return response.data;
    },

    /**
     * The fields that differ between a stored version and the current content.
     */
    async diffVersion(agentId: string, version: number): Promise<AgentVersionDiffResponse> {
      const response = await axios.get<AgentVersionDiffResponse>(
        ENDPOINTS.COACHES.VERSION_DIFF(agentId, version)
      );
      return response.data;
    },

    /**
     * Restore a stored version's content. The content it replaces is kept as a
     * new version, so a revert can itself be reverted.
     */
    async revertToVersion(agentId: string, version: number): Promise<RevertAgentVersionResponse> {
      const response = await axios.post<RevertAgentVersionResponse>(
        ENDPOINTS.COACHES.VERSION_REVERT(agentId, version)
      );
      return response.data;
    },

    /**
     * Submit one of the caller's own coaches to the Store. It waits in the
     * admin review queue until an admin approves or rejects it; only its
     * author may submit it.
     */
    async submitToStore(agentId: string): Promise<SubmitAgentForReviewResponse> {
      const response = await axios.post<SubmitAgentForReviewResponse>(ENDPOINTS.COACHES.SUBMIT(agentId));
      return response.data;
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
