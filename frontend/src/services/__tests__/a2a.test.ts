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

import { a2aApi } from '../api/a2a'

describe('a2aApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('registerA2AClient', () => {
    it('should register a new A2A client', async () => {
      const clientData = {
        name: 'Test Agent',
        description: 'Test description',
        capabilities: ['chat', 'tools'],
        contact_email: 'agent@example.com',
      }
      mockAxios.post.mockResolvedValue({ data: { id: 'client-1', ...clientData } })

      const result = await a2aApi.registerA2AClient(clientData)

      expect(mockAxios.post).toHaveBeenCalledWith('/a2a/clients', clientData)
      expect(result.id).toBe('client-1')
    })
  })

  describe('getA2AClients', () => {
    it('should fetch A2A clients list', async () => {
      const mockClients = [{ id: 'c1' }, { id: 'c2' }]
      mockAxios.get.mockResolvedValue({ data: mockClients })

      const result = await a2aApi.getA2AClients()

      expect(mockAxios.get).toHaveBeenCalledWith('/a2a/clients')
      expect(result).toEqual(mockClients)
    })
  })

  describe('deactivateA2AClient', () => {
    it('should deactivate an A2A client', async () => {
      mockAxios.delete.mockResolvedValue({ data: { success: true } })

      const result = await a2aApi.deactivateA2AClient('client-1')

      expect(mockAxios.delete).toHaveBeenCalledWith('/a2a/clients/client-1')
      expect(result.success).toBe(true)
    })
  })

  describe('getA2AClientUsage', () => {
    it('should fetch client usage without dates', async () => {
      mockAxios.get.mockResolvedValue({ data: { total: 100 } })

      await a2aApi.getA2AClientUsage('client-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/a2a/clients/client-1/usage?')
    })

    it('should fetch client usage with date range', async () => {
      mockAxios.get.mockResolvedValue({ data: { total: 50 } })

      await a2aApi.getA2AClientUsage('client-1', '2026-01-01', '2026-02-01')

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/a2a/clients/client-1/usage?start_date=2026-01-01&end_date=2026-02-01'
      )
    })
  })
})
