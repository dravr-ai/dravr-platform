// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Agent-to-Agent (A2A) protocol API methods - client registration, listing, usage, rate limits
// ABOUTME: Handles A2A protocol for agent-to-agent communication

import { axios } from './client';

export const a2aApi = {
  async registerA2AClient(data: {
    name: string;
    description: string;
    capabilities: string[];
    redirect_uris?: string[];
    contact_email: string;
    agent_version?: string;
    documentation_url?: string;
  }) {
    const response = await axios.post('/a2a/clients', data);
    return response.data;
  },

  async getA2AClients() {
    const response = await axios.get('/a2a/clients');
    return response.data;
  },

  async deactivateA2AClient(clientId: string) {
    const response = await axios.delete(`/a2a/clients/${clientId}`);
    return response.data;
  },

  async getA2AClientUsage(clientId: string, startDate?: string, endDate?: string) {
    const params = new URLSearchParams();
    if (startDate) params.append('start_date', startDate);
    if (endDate) params.append('end_date', endDate);

    const response = await axios.get(`/a2a/clients/${clientId}/usage?${params}`);
    return response.data;
  },

  async getA2AClientRateLimit(clientId: string) {
    const response = await axios.get(`/a2a/clients/${clientId}/rate-limit`);
    return response.data;
  },
};
