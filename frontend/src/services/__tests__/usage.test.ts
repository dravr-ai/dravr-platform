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

import { usageApi } from '../api/usage'

describe('usageApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('getStatus', () => {
    it('should fetch usage status', async () => {
      const mockStatus = {
        daily: {
          messages: { allowed: true, current: 5, limit: 100, warning: false, burst_zone: false },
        },
      }
      mockAxios.get.mockResolvedValue({ data: mockStatus })

      const result = await usageApi.getStatus()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/usage/status')
      expect(result).toEqual(mockStatus)
    })
  })

  describe('getLlmConsumption', () => {
    it('should fetch LLM consumption with default 30 days', async () => {
      const mockData = { summary: { total_tokens: 1000 } }
      mockAxios.get.mockResolvedValue({ data: mockData })

      const result = await usageApi.getLlmConsumption()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/usage/llm-consumption?days=30')
      expect(result).toEqual(mockData)
    })

    it('should fetch LLM consumption with custom params', async () => {
      mockAxios.get.mockResolvedValue({ data: {} })

      await usageApi.getLlmConsumption(7, 'model')

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/api/usage/llm-consumption?days=7&group_by=model'
      )
    })
  })

  describe('getAdminLlmConsumption', () => {
    it('should fetch admin LLM consumption with defaults', async () => {
      mockAxios.get.mockResolvedValue({ data: {} })

      await usageApi.getAdminLlmConsumption()

      expect(mockAxios.get).toHaveBeenCalledWith('/admin/usage/llm-consumption?days=30')
    })

    it('should fetch admin LLM consumption with tenant filter', async () => {
      mockAxios.get.mockResolvedValue({ data: {} })

      await usageApi.getAdminLlmConsumption(14, 'provider', 'tenant-1')

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/admin/usage/llm-consumption?days=14&group_by=provider&tenant_id=tenant-1'
      )
    })
  })
})
