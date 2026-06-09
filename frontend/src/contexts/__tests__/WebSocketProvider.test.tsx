// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests WebSocketProvider lifecycle: hidden-tab teardown and the absence
// ABOUTME: of auto-reconnect, which together bound how long an idle tab stays connected.

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, act } from '@testing-library/react'
import { WebSocketProvider } from '../WebSocketProvider'

const mockToken = 'test-jwt-token'
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ token: mockToken }),
}))

// Minimal controllable WebSocket double. Tracks every instance so tests can assert
// how many sockets were opened (i.e. whether a reconnect happened).
class MockWebSocket {
  static instances: MockWebSocket[] = []
  static OPEN = 1
  static CONNECTING = 0
  static CLOSED = 3
  readyState = MockWebSocket.CONNECTING
  onopen: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null
  onclose: ((event: { code: number }) => void) | null = null
  onerror: ((error: unknown) => void) | null = null
  close = vi.fn((code?: number) => {
    this.readyState = MockWebSocket.CLOSED
    this.onclose?.({ code: code ?? 1000 })
  })
  send = vi.fn()
  constructor(public url: string) {
    MockWebSocket.instances.push(this)
  }
  simulateOpen() {
    this.readyState = MockWebSocket.OPEN
    this.onopen?.()
  }
}

describe('WebSocketProvider connection lifecycle', () => {
  beforeEach(() => {
    MockWebSocket.instances = []
    vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket)
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  function setHidden(hidden: boolean) {
    Object.defineProperty(document, 'hidden', {
      configurable: true,
      get: () => hidden,
    })
    document.dispatchEvent(new Event('visibilitychange'))
  }

  it('closes the socket when the tab becomes hidden', () => {
    render(<WebSocketProvider><div /></WebSocketProvider>)
    expect(MockWebSocket.instances).toHaveLength(1)
    const ws = MockWebSocket.instances[0]
    act(() => ws.simulateOpen())

    act(() => setHidden(true))

    expect(ws.close).toHaveBeenCalledWith(1000, 'Provider disconnected')
  })

  it('does not auto-reconnect after an unexpected close', async () => {
    render(<WebSocketProvider><div /></WebSocketProvider>)
    const ws = MockWebSocket.instances[0]
    act(() => ws.simulateOpen())

    // Simulate the server dropping the connection (abnormal close code).
    act(() => ws.onclose?.({ code: 1006 }))

    // Advance well past the old 5s reconnect delay; no new socket must be opened.
    await act(async () => {
      vi.advanceTimersByTime(30_000)
    })

    expect(MockWebSocket.instances).toHaveLength(1)
  })

  it('does not reconnect on its own when the tab becomes visible again', () => {
    render(<WebSocketProvider><div /></WebSocketProvider>)
    const ws = MockWebSocket.instances[0]
    act(() => ws.simulateOpen())

    act(() => setHidden(true))
    expect(ws.close).toHaveBeenCalled()

    act(() => setHidden(false))

    // Reconnect is manual (via the banner button), so no new socket appears.
    expect(MockWebSocket.instances).toHaveLength(1)
  })
})
