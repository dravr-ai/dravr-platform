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

import { keysApi } from '../api/keys'

describe('keysApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('createApiKey', () => {
    it('should create an API key', async () => {
      const keyData = { name: 'Test Key', description: 'Test', rate_limit_requests: 1000 }
      const mockResponse = { data: { id: 'key-1', ...keyData } }
      mockAxios.post.mockResolvedValue(mockResponse)

      const result = await keysApi.createApiKey(keyData)

      expect(mockAxios.post).toHaveBeenCalledWith('/api/keys', keyData)
      expect(result).toEqual(mockResponse.data)
    })

    it('should support unlimited rate limit (0)', async () => {
      const keyData = { name: 'Unlimited Key', rate_limit_requests: 0 }
      mockAxios.post.mockResolvedValue({ data: keyData })

      await keysApi.createApiKey(keyData)

      expect(mockAxios.post).toHaveBeenCalledWith('/api/keys', keyData)
    })
  })

  describe('createTrialKey', () => {
    it('should create a trial key with preset limits', async () => {
      const mockResponse = { data: { id: 'trial-1' } }
      mockAxios.post.mockResolvedValue(mockResponse)

      await keysApi.createTrialKey({ name: 'Trial', description: 'Trial key' })

      expect(mockAxios.post).toHaveBeenCalledWith('/api/keys', {
        name: 'Trial',
        description: 'Trial key',
        rate_limit_requests: 1000,
        expires_in_days: 14,
      })
    })
  })

  describe('getApiKeys', () => {
    it('should fetch API keys list', async () => {
      const mockKeys = [{ id: 'key-1' }, { id: 'key-2' }]
      mockAxios.get.mockResolvedValue({ data: mockKeys })

      const result = await keysApi.getApiKeys()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/keys')
      expect(result).toEqual(mockKeys)
    })
  })

  describe('deactivateApiKey', () => {
    it('should deactivate an API key by ID', async () => {
      mockAxios.delete.mockResolvedValue({ data: { success: true } })

      const result = await keysApi.deactivateApiKey('key-1')

      expect(mockAxios.delete).toHaveBeenCalledWith('/api/keys/key-1')
      expect(result).toEqual({ success: true })
    })
  })

  describe('getApiKeyUsage', () => {
    it('should fetch usage without date params', async () => {
      mockAxios.get.mockResolvedValue({ data: { total_requests: 100 } })

      const result = await keysApi.getApiKeyUsage('key-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/api/keys/key-1/usage?')
      expect(result).toEqual({ total_requests: 100 })
    })

    it('should fetch usage with date range', async () => {
      mockAxios.get.mockResolvedValue({ data: { total_requests: 50 } })

      await keysApi.getApiKeyUsage('key-1', '2026-01-01', '2026-02-01')

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/api/keys/key-1/usage?start_date=2026-01-01&end_date=2026-02-01'
      )
    })
  })
})
