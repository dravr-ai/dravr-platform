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

  describe('getA2AClient', () => {
    it('should fetch a single A2A client', async () => {
      mockAxios.get.mockResolvedValue({ data: { id: 'client-1', name: 'Agent' } })

      const result = await a2aApi.getA2AClient('client-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/a2a/clients/client-1')
      expect(result.name).toBe('Agent')
    })
  })

  describe('updateA2AClient', () => {
    it('should update an A2A client', async () => {
      mockAxios.put.mockResolvedValue({ data: { id: 'client-1', name: 'Updated Agent' } })

      const result = await a2aApi.updateA2AClient('client-1', { name: 'Updated Agent' })

      expect(mockAxios.put).toHaveBeenCalledWith('/a2a/clients/client-1', {
        name: 'Updated Agent',
      })
      expect(result.name).toBe('Updated Agent')
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

  describe('getA2ASessions', () => {
    it('should fetch sessions without filter', async () => {
      mockAxios.get.mockResolvedValue({ data: { sessions: [] } })

      await a2aApi.getA2ASessions()

      expect(mockAxios.get).toHaveBeenCalledWith('/a2a/sessions?')
    })

    it('should fetch sessions filtered by client', async () => {
      mockAxios.get.mockResolvedValue({ data: { sessions: [] } })

      await a2aApi.getA2ASessions('client-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/a2a/sessions?client_id=client-1')
    })
  })

  describe('getA2ADashboardOverview', () => {
    it('should fetch A2A dashboard overview', async () => {
      mockAxios.get.mockResolvedValue({ data: { total_clients: 5 } })

      const result = await a2aApi.getA2ADashboardOverview()

      expect(mockAxios.get).toHaveBeenCalledWith('/a2a/dashboard/overview')
      expect(result.total_clients).toBe(5)
    })
  })

  describe('getA2AAgentCard', () => {
    it('should fetch agent card', async () => {
      mockAxios.get.mockResolvedValue({ data: { name: 'Pierre Agent' } })

      const result = await a2aApi.getA2AAgentCard()

      expect(mockAxios.get).toHaveBeenCalledWith('/a2a/agent-card')
      expect(result.name).toBe('Pierre Agent')
    })
  })
})
