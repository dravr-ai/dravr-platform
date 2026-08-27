// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import '@testing-library/jest-dom'
import { initI18n } from '@pierre/i18n'

// Mock fetch for API calls
global.fetch = vi.fn()

// Polyfill matchMedia — jsdom does not implement it, so hooks/tests that read or
// spy on window.matchMedia (e.g. useBreakpoint) get a real function in every test
// file. Without this, the test only passes when another file happens to leak a
// matchMedia into the shared worker first (flaky across CI test ordering).
if (typeof window !== 'undefined' && !window.matchMedia) {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList
}

// Polyfill scrollIntoView — jsdom does not implement it, so any test that renders
// a list which auto-scrolls to its bottom sentinel (MessageList) throws out of a
// passive effect, which surfaces as an unhandled error rather than a test failure.
if (typeof Element !== 'undefined' && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {}
}

// Mock IntersectionObserver (not available in jsdom)
class MockIntersectionObserver implements IntersectionObserver {
  root: Element | Document | null = null
  rootMargin: string = ''
  thresholds: ReadonlyArray<number> = []

  constructor(private callback: IntersectionObserverCallback) {
    // Store callback for potential manual triggering in tests
    void this.callback
  }

  observe(target: Element): void {
    // No-op in test environment - parameter intentionally unused
    void target
  }

  unobserve(target: Element): void {
    // No-op in test environment - parameter intentionally unused
    void target
  }

  disconnect(): void {
    // No-op in test environment
  }

  takeRecords(): IntersectionObserverEntry[] {
    return []
  }
}

global.IntersectionObserver = MockIntersectionObserver

// Mock WebSocket
class MockWebSocket {
  url: string
  readyState: number = WebSocket.CONNECTING
  onopen: ((event: Event) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null

  constructor(url: string) {
    this.url = url
    // Simulate connection after a tick
    setTimeout(() => {
      this.readyState = WebSocket.OPEN
      if (this.onopen) {
        this.onopen(new Event('open'))
      }
    }, 0)
  }

  send(_data: string) {
    // Mock send - parameter intentionally unused
    void _data;
  }

  close() {
    this.readyState = WebSocket.CLOSED
    if (this.onclose) {
      this.onclose(new CloseEvent('close'))
    }
  }
}

global.WebSocket = MockWebSocket as typeof WebSocket

// Mock Chart.js
vi.mock('chart.js', () => ({
  Chart: {
    register: vi.fn(),
  },
  CategoryScale: vi.fn(),
  LinearScale: vi.fn(),
  PointElement: vi.fn(),
  LineElement: vi.fn(),
  BarElement: vi.fn(),
  Title: vi.fn(),
  Tooltip: vi.fn(),
  Legend: vi.fn(),
  ArcElement: vi.fn(),
}))

vi.mock('react-chartjs-2', () => ({
  Line: vi.fn(() => 'Line Chart'),
  Bar: vi.fn(() => 'Bar Chart'),
  Doughnut: vi.fn(() => 'Doughnut Chart'),
}))
// Initialize i18next for every test file. In production main.tsx does this
// before the first render, so a component calling useTranslation() can assume
// a live instance; without it here, any test rendering translated chrome
// crashes on an undefined i18n. The persister rejects rather than no-opping:
// a test that changes language must register the writer it means to assert,
// so a missing client→server wire fails loudly instead of passing quietly.
// Unit tests assert English copy, so they pin the locale rather than inherit
// the product default (French). A test that means to exercise another locale
// calls `i18n.changeLanguage` itself.
await initI18n({
  persistLocale: () =>
    Promise.reject(
      new Error('No locale persister registered for this test — call initI18n({ persistLocale }).'),
    ),
  config: { lng: 'en' },
})
