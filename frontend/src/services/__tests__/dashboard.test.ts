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
})
