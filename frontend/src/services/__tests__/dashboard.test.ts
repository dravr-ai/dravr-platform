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

  describe('getDashboardOverview', () => {
    it('should fetch dashboard overview', async () => {
      const mockData = { total_requests: 100, active_keys: 5 }
      mockAxios.get.mockResolvedValue({ data: mockData })

      const result = await dashboardApi.getDashboardOverview()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/overview')
      expect(result).toEqual(mockData)
    })
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

  describe('getRateLimitOverview', () => {
    it('should fetch rate limit overview', async () => {
      const mockData = { rate_limits: [] }
      mockAxios.get.mockResolvedValue({ data: mockData })

      const result = await dashboardApi.getRateLimitOverview()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/rate-limits')
      expect(result).toEqual(mockData)
    })
  })

  describe('getRequestLogs', () => {
    it('should fetch request logs without filters', async () => {
      mockAxios.get.mockResolvedValue({ data: { logs: [] } })

      await dashboardApi.getRequestLogs()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/request-logs?')
    })

    it('should fetch request logs with API key filter', async () => {
      mockAxios.get.mockResolvedValue({ data: { logs: [] } })

      await dashboardApi.getRequestLogs('key-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/request-logs?api_key_id=key-1')
    })

    it('should fetch request logs with all filters', async () => {
      mockAxios.get.mockResolvedValue({ data: { logs: [] } })

      await dashboardApi.getRequestLogs('key-1', {
        timeRange: '1h',
        status: 'success',
        tool: 'chat',
      })

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/api/dashboard/request-logs?api_key_id=key-1&time_range=1h&status=success&tool=chat'
      )
    })

    it('should not include status when set to all', async () => {
      mockAxios.get.mockResolvedValue({ data: { logs: [] } })

      await dashboardApi.getRequestLogs(undefined, {
        timeRange: '1h',
        status: 'all',
        tool: 'all',
      })

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/request-logs?time_range=1h')
    })
  })

  describe('getRequestStats', () => {
    it('should fetch request stats with defaults', async () => {
      mockAxios.get.mockResolvedValue({ data: { total: 100 } })

      await dashboardApi.getRequestStats()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/dashboard/request-stats?time_range=1h')
    })

    it('should fetch request stats with custom params', async () => {
      mockAxios.get.mockResolvedValue({ data: { total: 50 } })

      await dashboardApi.getRequestStats('key-1', '7d')

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/api/dashboard/request-stats?api_key_id=key-1&time_range=7d'
      )
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
