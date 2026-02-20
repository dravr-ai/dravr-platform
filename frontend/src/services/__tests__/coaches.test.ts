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

import { coachesApi } from '../api/coaches'

describe('coachesApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('getCoaches', () => {
    it('should fetch coaches without options', async () => {
      const mockData = { coaches: [], total: 0 }
      mockAxios.get.mockResolvedValue({ data: mockData })

      const result = await coachesApi.getCoaches()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/coaches')
      expect(result).toEqual(mockData)
    })

    it('should include query params when options provided', async () => {
      mockAxios.get.mockResolvedValue({ data: { coaches: [] } })

      await coachesApi.getCoaches({
        category: 'running',
        favorites_only: true,
        limit: 10,
        offset: 5,
      })

      expect(mockAxios.get).toHaveBeenCalledWith(
        '/api/coaches?category=running&favorites_only=true&limit=10&offset=5'
      )
    })

    it('should include include_hidden param', async () => {
      mockAxios.get.mockResolvedValue({ data: { coaches: [] } })

      await coachesApi.getCoaches({ include_hidden: true })

      expect(mockAxios.get).toHaveBeenCalledWith('/api/coaches?include_hidden=true')
    })
  })

  describe('toggleCoachFavorite', () => {
    it('should toggle favorite status', async () => {
      mockAxios.post.mockResolvedValue({ data: { is_favorite: true } })

      const result = await coachesApi.toggleCoachFavorite('coach-1')

      expect(mockAxios.post).toHaveBeenCalledWith('/api/coaches/coach-1/favorite')
      expect(result.is_favorite).toBe(true)
    })
  })

  describe('createCoach', () => {
    it('should create a coach', async () => {
      const coachData = {
        title: 'Running Coach',
        description: 'For runners',
        system_prompt: 'You are a running coach',
        category: 'running',
        tags: ['marathon', 'training'],
      }
      mockAxios.post.mockResolvedValue({ data: { id: 'coach-1', ...coachData } })

      const result = await coachesApi.createCoach(coachData)

      expect(mockAxios.post).toHaveBeenCalledWith('/api/coaches', coachData)
      expect(result.title).toBe('Running Coach')
    })
  })

  describe('updateCoach', () => {
    it('should update a coach', async () => {
      mockAxios.put.mockResolvedValue({ data: { id: 'coach-1', title: 'Updated' } })

      const result = await coachesApi.updateCoach('coach-1', { title: 'Updated' })

      expect(mockAxios.put).toHaveBeenCalledWith('/api/coaches/coach-1', { title: 'Updated' })
      expect(result.title).toBe('Updated')
    })
  })

  describe('deleteCoach', () => {
    it('should delete a coach', async () => {
      mockAxios.delete.mockResolvedValue({})

      await coachesApi.deleteCoach('coach-1')

      expect(mockAxios.delete).toHaveBeenCalledWith('/api/coaches/coach-1')
    })
  })

  describe('hideCoach', () => {
    it('should hide a coach', async () => {
      mockAxios.post.mockResolvedValue({ data: { success: true, is_hidden: true } })

      const result = await coachesApi.hideCoach('coach-1')

      expect(mockAxios.post).toHaveBeenCalledWith('/api/coaches/coach-1/hide')
      expect(result.is_hidden).toBe(true)
    })
  })

  describe('showCoach', () => {
    it('should show a hidden coach', async () => {
      mockAxios.delete.mockResolvedValue({ data: { success: true, is_hidden: false } })

      const result = await coachesApi.showCoach('coach-1')

      expect(mockAxios.delete).toHaveBeenCalledWith('/api/coaches/coach-1/hide')
      expect(result.is_hidden).toBe(false)
    })
  })

  describe('forkCoach', () => {
    it('should fork a coach', async () => {
      mockAxios.post.mockResolvedValue({ data: { coach: { id: 'coach-2' } } })

      const result = await coachesApi.forkCoach('coach-1')

      expect(mockAxios.post).toHaveBeenCalledWith('/api/coaches/coach-1/fork')
      expect(result.coach.id).toBe('coach-2')
    })
  })

  describe('getCoachVersions', () => {
    it('should fetch versions without limit', async () => {
      mockAxios.get.mockResolvedValue({ data: { versions: [], current_version: 1 } })

      await coachesApi.getCoachVersions('coach-1')

      expect(mockAxios.get).toHaveBeenCalledWith('/api/coaches/coach-1/versions')
    })

    it('should fetch versions with limit', async () => {
      mockAxios.get.mockResolvedValue({ data: { versions: [] } })

      await coachesApi.getCoachVersions('coach-1', 5)

      expect(mockAxios.get).toHaveBeenCalledWith('/api/coaches/coach-1/versions?limit=5')
    })
  })

  describe('revertCoachToVersion', () => {
    it('should revert coach to a specific version', async () => {
      mockAxios.post.mockResolvedValue({
        data: { coach: { id: 'coach-1' }, reverted_to_version: 2, new_version: 4 },
      })

      const result = await coachesApi.revertCoachToVersion('coach-1', 2)

      expect(mockAxios.post).toHaveBeenCalledWith('/api/coaches/coach-1/versions/2/revert')
      expect(result.reverted_to_version).toBe(2)
    })
  })

  describe('getPromptSuggestions', () => {
    it('should fetch prompt suggestions', async () => {
      const mockData = { categories: [], welcome_prompt: 'Hello!' }
      mockAxios.get.mockResolvedValue({ data: mockData })

      const result = await coachesApi.getPromptSuggestions()

      expect(mockAxios.get).toHaveBeenCalledWith('/api/prompts/suggestions')
      expect(result.welcome_prompt).toBe('Hello!')
    })
  })
})
