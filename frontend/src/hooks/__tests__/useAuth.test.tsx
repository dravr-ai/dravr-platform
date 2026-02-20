// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect } from 'vitest'
import { renderHook } from '@testing-library/react'
import { useAuth } from '../useAuth'
import { AuthContext } from '../../contexts/auth'
import type { ReactNode } from 'react'

const mockAuthContext = {
  user: { id: '1', email: 'test@example.com', display_name: 'Test User' },
  token: 'mock-token',
  isAuthenticated: true,
  isLoading: false,
  loading: false,
  login: vi.fn(),
  loginWithFirebase: vi.fn(),
  logout: vi.fn(),
  impersonation: {
    isImpersonating: false,
    targetUser: null,
    sessionId: null,
    originalUser: null,
  },
  startImpersonation: vi.fn(),
  endImpersonation: vi.fn(),
}

function createWrapper(value = mockAuthContext) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <AuthContext.Provider value={value}>
        {children}
      </AuthContext.Provider>
    )
  }
}

describe('useAuth hook', () => {
  it('should return auth context when used within AuthProvider', () => {
    const { result } = renderHook(() => useAuth(), {
      wrapper: createWrapper(),
    })

    expect(result.current.user).toEqual(mockAuthContext.user)
    expect(result.current.isAuthenticated).toBe(true)
    expect(result.current.token).toBe('mock-token')
  })

  it('should throw error when used outside AuthProvider', () => {
    expect(() => {
      renderHook(() => useAuth())
    }).toThrow('useAuth must be used within an AuthProvider')
  })

  it('should provide login function', () => {
    const { result } = renderHook(() => useAuth(), {
      wrapper: createWrapper(),
    })

    expect(typeof result.current.login).toBe('function')
  })

  it('should provide logout function', () => {
    const { result } = renderHook(() => useAuth(), {
      wrapper: createWrapper(),
    })

    expect(typeof result.current.logout).toBe('function')
  })

  it('should provide impersonation state', () => {
    const { result } = renderHook(() => useAuth(), {
      wrapper: createWrapper(),
    })

    expect(result.current.impersonation.isImpersonating).toBe(false)
    expect(result.current.impersonation.targetUser).toBeNull()
  })

  it('should reflect unauthenticated state', () => {
    const unauthContext = {
      ...mockAuthContext,
      user: null,
      token: null,
      isAuthenticated: false,
    }
    const { result } = renderHook(() => useAuth(), {
      wrapper: createWrapper(unauthContext),
    })

    expect(result.current.user).toBeNull()
    expect(result.current.isAuthenticated).toBe(false)
    expect(result.current.token).toBeNull()
  })
})
