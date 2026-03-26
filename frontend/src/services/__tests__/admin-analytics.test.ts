// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for admin analytics API methods
// ABOUTME: Verifies overview, conversations, coaches, and engagement endpoints

import { describe, it, expect, beforeEach, vi } from 'vitest';

const { mockAxios } = vi.hoisted(() => ({
  mockAxios: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}));

vi.mock('../api/client', () => ({
  axios: mockAxios,
}));

import { adminAnalyticsApi } from '../api/admin-analytics';

describe('adminAnalyticsApi', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('getOverview', () => {
    it('should fetch analytics overview', async () => {
      const mockData = {
        active_users_24h: 12,
        active_users_7d: 45,
        conversations_today: 8,
        messages_today: 42,
        llm_spend_today_usd: 1.25,
        llm_spend_7d_usd: 8.50,
        coach_adoption_pct: 67,
        connected_providers: 3,
        total_users: 50,
        total_coaches: 15,
      };
      mockAxios.get.mockResolvedValue({ data: mockData });

      const result = await adminAnalyticsApi.getOverview();

      expect(mockAxios.get).toHaveBeenCalledWith('/api/admin/analytics/overview');
      expect(result).toEqual(mockData);
    });
  });

  describe('getConversations', () => {
    it('should fetch conversation analytics with default 7 days', async () => {
      const mockData = { daily: [], peak_hour: 14, avg_messages_per_conversation: 5.2, total_conversations: 0, total_messages: 0 };
      mockAxios.get.mockResolvedValue({ data: mockData });

      const result = await adminAnalyticsApi.getConversations();

      expect(mockAxios.get).toHaveBeenCalledWith('/api/admin/analytics/conversations?days=7');
      expect(result).toEqual(mockData);
    });

    it('should fetch conversation analytics with custom days', async () => {
      mockAxios.get.mockResolvedValue({ data: { daily: [] } });

      await adminAnalyticsApi.getConversations(30);

      expect(mockAxios.get).toHaveBeenCalledWith('/api/admin/analytics/conversations?days=30');
    });
  });

  describe('getCoaches', () => {
    it('should fetch coach analytics', async () => {
      const mockData = {
        top_coaches: [
          { coach_id: 'c1', title: 'Endurance', category: 'training', use_count: 42, is_system: true },
        ],
        categories: [
          { category: 'training', count: 5 },
          { category: 'nutrition', count: 3 },
        ],
        system_coach_count: 8,
        user_coach_count: 4,
      };
      mockAxios.get.mockResolvedValue({ data: mockData });

      const result = await adminAnalyticsApi.getCoaches();

      expect(mockAxios.get).toHaveBeenCalledWith('/api/admin/analytics/coaches');
      expect(result).toEqual(mockData);
    });
  });

  describe('getEngagement', () => {
    it('should fetch engagement analytics with default 7 days', async () => {
      const mockData = {
        dau_series: [{ date: '2026-03-25', dau: 10 }],
        engagement_tiers: [{ tier: 'active', count: 20 }],
        provider_distribution: [{ provider: 'strava', connected_count: 15 }],
        group_stats: { total_groups: 3, total_members: 25 },
      };
      mockAxios.get.mockResolvedValue({ data: mockData });

      const result = await adminAnalyticsApi.getEngagement();

      expect(mockAxios.get).toHaveBeenCalledWith('/api/admin/analytics/engagement?days=7');
      expect(result).toEqual(mockData);
    });

    it('should fetch engagement analytics with custom days', async () => {
      mockAxios.get.mockResolvedValue({ data: { dau_series: [] } });

      await adminAnalyticsApi.getEngagement(90);

      expect(mockAxios.get).toHaveBeenCalledWith('/api/admin/analytics/engagement?days=90');
    });
  });
});
