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

import { chatApi } from '../api/chat'

describe('chatApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('getConversations', () => {
    it('should fetch conversations with defaults', async () => {
      const mockData = { conversations: [], total: 0, limit: 50, offset: 0 }
      mockAxios.get.mockResolvedValue({ data: mockData })

      const result = await chatApi.getConversations()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/chat/conversations?limit=50&offset=0')
      expect(result).toEqual(mockData)
    })

    it('should fetch conversations with custom pagination', async () => {
      mockAxios.get.mockResolvedValue({ data: { conversations: [] } })

      await chatApi.getConversations(10, 20)

      expect(mockAxios.get).toHaveBeenCalledWith('/api/chat/conversations?limit=10&offset=20')
    })
  })

  describe('createConversation', () => {
    it('should create a new conversation', async () => {
      const mockConversation = { id: 'conv-1', title: 'Test Conversation' }
      mockAxios.post.mockResolvedValue({ data: mockConversation })

      const result = await chatApi.createConversation({ title: 'Test Conversation' })

      expect(mockAxios.post).toHaveBeenCalledWith('/api/chat/conversations', {
        title: 'Test Conversation',
      })
      expect(result).toEqual(mockConversation)
    })

    it('should create conversation with model and system prompt', async () => {
      mockAxios.post.mockResolvedValue({ data: { id: 'conv-1' } })

      await chatApi.createConversation({
        title: 'Custom Chat',
        model: 'gpt-4',
        system_prompt: 'You are a fitness coach',
      })

      expect(mockAxios.post).toHaveBeenCalledWith('/api/chat/conversations', {
        title: 'Custom Chat',
        model: 'gpt-4',
        system_prompt: 'You are a fitness coach',
      })
    })
  })

  describe('getConversation', () => {
    it('should fetch a single conversation', async () => {
      mockAxios.get.mockResolvedValue({ data: { id: 'conv-1', title: 'Chat' } })

      const result = await chatApi.getConversation('conv-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/api/chat/conversations/conv-1')
      expect(result.id).toBe('conv-1')
    })
  })

  describe('updateConversation', () => {
    it('should update conversation title', async () => {
      mockAxios.put.mockResolvedValue({ data: { id: 'conv-1', title: 'Updated' } })

      const result = await chatApi.updateConversation('conv-1', { title: 'Updated' })

      expect(mockAxios.put).toHaveBeenCalledWith('/api/chat/conversations/conv-1', {
        title: 'Updated',
      })
      expect(result.title).toBe('Updated')
    })
  })

  describe('deleteConversation', () => {
    it('should delete a conversation', async () => {
      mockAxios.delete.mockResolvedValue({})

      await chatApi.deleteConversation('conv-1')

      expect(mockAxios.delete).toHaveBeenCalledWith('/api/chat/conversations/conv-1')
    })
  })

  describe('getConversationMessages', () => {
    it('should fetch messages for a conversation', async () => {
      const mockMessages = {
        messages: [
          { id: 'msg-1', role: 'user', content: 'Hello' },
          { id: 'msg-2', role: 'assistant', content: 'Hi there!' },
        ],
      }
      mockAxios.get.mockResolvedValue({ data: mockMessages })

      const result = await chatApi.getConversationMessages('conv-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/api/chat/conversations/conv-1/messages')
      expect(result.messages).toHaveLength(2)
      expect(result.messages[0].role).toBe('user')
    })
  })
})
