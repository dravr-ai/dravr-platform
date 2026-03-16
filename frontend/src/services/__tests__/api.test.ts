// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that the API barrel exports all required domain modules
// ABOUTME: Verifies shared @pierre/api-client modules and web-only modules are accessible

import { describe, it, expect, vi } from 'vitest'

// Mock the @pierre/api-client package and web adapter
vi.mock('@pierre/api-client', () => {
  const mockAxiosInstance = {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: { use: vi.fn() },
      response: { use: vi.fn() },
    },
  }
  return {
    createPierreApi: vi.fn(() => ({
      auth: { login: vi.fn(), register: vi.fn() },
      chat: { getConversations: vi.fn() },
      coaches: { list: vi.fn() },
      oauth: {
        getStatus: vi.fn(),
        getAuthorizeUrl: vi.fn(),
        getProvidersStatus: vi.fn(),
        disconnectProvider: vi.fn(),
        getAuthorizeUrlForProvider: vi.fn(),
      },
      social: { listFriends: vi.fn(), getInsightSuggestions: vi.fn(), shareFromActivity: vi.fn() },
      store: { browse: vi.fn() },
      user: { getLlmSettings: vi.fn(), saveLlmCredentials: vi.fn() },
      notifications: { getNotifications: vi.fn() },
      axios: mockAxiosInstance,
      adapter: {
        authStorage: {
          setCsrfToken: vi.fn(), getCsrfToken: vi.fn(),
          setUser: vi.fn(), getUser: vi.fn(), clear: vi.fn(),
          getToken: vi.fn(), setToken: vi.fn(), removeToken: vi.fn(),
          getRefreshToken: vi.fn(), setRefreshToken: vi.fn(),
        },
        httpConfig: { baseURL: '' },
        authFailure: { onAuthFailure: vi.fn() },
      },
    })),
  }
})

vi.mock('@pierre/api-client/adapters/web', () => ({
  createWebAdapter: vi.fn(() => ({
    authStorage: {
      setCsrfToken: vi.fn(), getCsrfToken: vi.fn(),
      setUser: vi.fn(), getUser: vi.fn(), clear: vi.fn(),
      getToken: vi.fn(), setToken: vi.fn(), removeToken: vi.fn(),
      getRefreshToken: vi.fn(), setRefreshToken: vi.fn(),
    },
    httpConfig: { baseURL: '' },
    authFailure: { onAuthFailure: vi.fn() },
  })),
}))

vi.mock('axios', () => {
  const mockAxiosInstance = {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
    interceptors: {
      request: { use: vi.fn() },
      response: { use: vi.fn() },
    },
  }
  return {
    default: {
      ...mockAxiosInstance,
      create: vi.fn(() => mockAxiosInstance),
      defaults: {
        baseURL: '',
        withCredentials: true,
        headers: { common: {} },
      },
    },
  }
})

import {
  authApi,
  chatApi,
  coachesApi,
  oauthApi,
  socialApi,
  storeApi,
  userApi,
  providersApi,
  keysApi,
  dashboardApi,
  a2aApi,
  adminApi,
} from '../api/index'

describe('API Barrel Exports', () => {
  it('should export all shared domain APIs', () => {
    expect(authApi).toBeDefined()
    expect(chatApi).toBeDefined()
    expect(coachesApi).toBeDefined()
    expect(oauthApi).toBeDefined()
    expect(socialApi).toBeDefined()
    expect(storeApi).toBeDefined()
    expect(userApi).toBeDefined()
  })

  it('should export providersApi delegating to shared oauth', () => {
    expect(providersApi).toBeDefined()
    expect(typeof providersApi.getProvidersStatus).toBe('function')
  })

  it('should export web-only domain APIs', () => {
    expect(keysApi).toBeDefined()
    expect(dashboardApi).toBeDefined()
    expect(a2aApi).toBeDefined()
    expect(adminApi).toBeDefined()
  })
})
