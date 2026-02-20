// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect } from 'vitest'
import { renderHook } from '@testing-library/react'
import { useWebSocketContext } from '../useWebSocketContext'
import { WebSocketContext, type WebSocketContextType } from '../../contexts/WebSocketContext'
import type { ReactNode } from 'react'

const mockWebSocketContext: WebSocketContextType = {
  isConnected: true,
  lastMessage: null,
  sendMessage: vi.fn(),
  subscribe: vi.fn(),
  reconnect: vi.fn(),
}

function createWrapper(value: WebSocketContextType = mockWebSocketContext) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <WebSocketContext.Provider value={value}>
        {children}
      </WebSocketContext.Provider>
    )
  }
}

describe('useWebSocketContext hook', () => {
  it('should return WebSocket context when used within provider', () => {
    const { result } = renderHook(() => useWebSocketContext(), {
      wrapper: createWrapper(),
    })

    expect(result.current.isConnected).toBe(true)
    expect(result.current.lastMessage).toBeNull()
    expect(typeof result.current.sendMessage).toBe('function')
    expect(typeof result.current.subscribe).toBe('function')
    expect(typeof result.current.reconnect).toBe('function')
  })

  it('should throw error when used outside provider', () => {
    expect(() => {
      renderHook(() => useWebSocketContext())
    }).toThrow('useWebSocketContext must be used within a WebSocketProvider')
  })

  it('should reflect disconnected state', () => {
    const disconnectedContext: WebSocketContextType = {
      ...mockWebSocketContext,
      isConnected: false,
    }
    const { result } = renderHook(() => useWebSocketContext(), {
      wrapper: createWrapper(disconnectedContext),
    })

    expect(result.current.isConnected).toBe(false)
  })

  it('should provide last message when available', () => {
    const contextWithMessage: WebSocketContextType = {
      ...mockWebSocketContext,
      lastMessage: {
        type: 'usage_update',
        requests_today: 42,
      },
    }
    const { result } = renderHook(() => useWebSocketContext(), {
      wrapper: createWrapper(contextWithMessage),
    })

    expect(result.current.lastMessage).not.toBeNull()
    expect(result.current.lastMessage?.type).toBe('usage_update')
    expect(result.current.lastMessage?.requests_today).toBe(42)
  })
})
