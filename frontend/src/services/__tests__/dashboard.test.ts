// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'

const { mockAxios } = vi.hoisted(() => ({
  mockAxios: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../api/client', () => ({
  axios: mockAxios,
}))

import { dashboardApi } from '../api/dashboard'

describe('dashboardApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('getUsageAnalytics', () => {
    it('should fetch analytics with default 30 days', async () => {
      mockAxios.get.mockResolvedValue({ data: { daily: [] } })

      await dashboardApi.getUsageAnalytics()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/analytics?days=30')
    })

    it('should fetch analytics with custom days', async () => {
      mockAxios.get.mockResolvedValue({ data: { daily: [] } })

      await dashboardApi.getUsageAnalytics(7)

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/analytics?days=7')
    })
  })

  describe('getToolUsageBreakdown', () => {
    it('should fetch tool usage with defaults', async () => {
      mockAxios.get.mockResolvedValue({ data: { tools: [] } })

      await dashboardApi.getToolUsageBreakdown()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/tool-usage?time_range=7d')
    })

    it('should fetch tool usage with custom params', async () => {
      mockAxios.get.mockResolvedValue({ data: { tools: [] } })

      await dashboardApi.getToolUsageBreakdown('key-1', '30d')

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/api/dashboard/tool-usage?api_key_id=key-1&time_range=30d'
      )
    })
  })
})
