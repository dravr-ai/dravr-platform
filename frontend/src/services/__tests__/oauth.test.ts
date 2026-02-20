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

import { oauthApi } from '../api/oauth'

describe('oauthApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('getOAuthStatus', () => {
    it('should fetch OAuth status and wrap array response', async () => {
      const mockProviders = [
        { provider: 'strava', connected: true, last_sync: '2026-02-20T10:00:00Z' },
        { provider: 'garmin', connected: false, last_sync: null },
      ]
      mockAxios.get.mockResolvedValue({ data: mockProviders })

      const result = await oauthApi.getOAuthStatus()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/oauth/status')
      expect(result.providers).toEqual(mockProviders)
      expect(result.providers).toHaveLength(2)
    })
  })

  describe('getProvidersStatus', () => {
    it('should fetch all provider statuses', async () => {
      const mockData = {
        providers: [
          {
            provider: 'strava',
            display_name: 'Strava',
            requires_oauth: true,
            connected: true,
            capabilities: ['activities', 'athlete'],
          },
          {
            provider: 'synthetic',
            display_name: 'Synthetic',
            requires_oauth: false,
            connected: true,
            capabilities: ['activities'],
          },
        ],
      }
      mockAxios.get.mockResolvedValue({ data: mockData })

      const result = await oauthApi.getProvidersStatus()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/providers')
      expect(result.providers).toHaveLength(2)
      expect(result.providers[0].requires_oauth).toBe(true)
      expect(result.providers[1].requires_oauth).toBe(false)
    })
  })

  describe('getOAuthAuthorizeUrl', () => {
    it('should fetch authorization URL for a provider', async () => {
      const mockUrl = 'https://www.strava.com/oauth/authorize?client_id=123'
      mockAxios.get.mockResolvedValue({ data: { authorization_url: mockUrl } })

      const result = await oauthApi.getOAuthAuthorizeUrl('strava')

      expect(mockAxios.get).toHaveBeenCalledWith('/api/oauth/mobile/init/strava')
      expect(result).toBe(mockUrl)
    })
  })

  describe('disconnectProvider', () => {
    it('should disconnect a provider', async () => {
      mockAxios.delete.mockResolvedValue({})

      await oauthApi.disconnectProvider('strava')

      expect(mockAxios.delete).toHaveBeenCalledWith('/api/oauth/providers/strava/disconnect')
    })
  })
})
