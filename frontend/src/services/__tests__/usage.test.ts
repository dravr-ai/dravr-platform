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

import type { AxiosInstance } from 'axios'
import { createUsageApi } from '@pierre/api-client'
import type { LimitCheckResult, UsageStatusResponse } from '@pierre/shared-types'
import { adminUsageApi } from '../api/usage'

const counter = (current: number, limit: number): LimitCheckResult => ({
  allowed: current < limit,
  current,
  limit,
  warning: current >= limit * 0.8,
  burst_zone: false,
  resets_at: '2026-09-26T00:00:00Z',
})

describe('shared usage domain', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('reads the calling user\'s counters from /api/usage/status', async () => {
    const status: UsageStatusResponse = {
      daily: { messages: counter(45, 50), tokens: counter(1200, 100000), tool_calls: counter(3, 200) },
      weekly: { messages: counter(120, 300), tokens: counter(9000, 500000), tool_calls: counter(10, 1000) },
      resources: { conversations: 12, max_conversations: 100, agents: 2, max_agents: 5 },
    }
    mockAxios.get.mockResolvedValue({ data: status })

    const result = await createUsageApi(mockAxios as unknown as AxiosInstance).getStatus()

    expect(mockAxios.get).toHaveBeenCalledWith('/api/usage/status')
    expect(result.daily.messages.current).toBe(45)
    expect(result.daily.messages.warning).toBe(true)
    expect(result.resources.max_agents).toBe(5)
  })
})

describe('adminUsageApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('getAdminLlmConsumption', () => {
    it('should fetch admin LLM consumption with defaults', async () => {
      mockAxios.get.mockResolvedValue({ data: {} })

      await adminUsageApi.getAdminLlmConsumption()

      expect(mockAxios.get).toHaveBeenCalledWith('/admin/usage/llm-consumption?days=30')
    })

    it('should fetch admin LLM consumption with tenant filter', async () => {
      mockAxios.get.mockResolvedValue({ data: {} })

      await adminUsageApi.getAdminLlmConsumption(14, 'provider', 'tenant-1')

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/admin/usage/llm-consumption?days=14&group_by=provider&tenant_id=tenant-1'
      )
    })
  })
})
