// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the PostHog distinct id the web app identifies a user with, and that nothing is sent before it is set
// ABOUTME: The same id the server derives with hash_id, so a coach's web and server events are one PostHog person (carnet#848)

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import posthog from 'posthog-js'
import { bootAnalytics, shutdownAnalytics, track } from '../analytics'

vi.mock('posthog-js', () => ({
  default: {
    init: vi.fn(),
    identify: vi.fn(),
    capture: vi.fn(),
    reset: vi.fn(),
    opt_out_capturing: vi.fn(),
  },
}))

// The same pair is pinned by `hash_id` in crates/pierre-services/src/analytics.rs,
// by the phone's analytics test and by the shared helper's own test.
const USER_ID = '123e4567-e89b-12d3-a456-426614174000'
const DISTINCT_ID = '986c0dc956dc822b5d8f698661b9eb1ef880786ff9043c16744d2a420e99e9bb'

describe('web analytics identity', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.stubEnv('VITE_POSTHOG_API_KEY', 'phc_test_key')
  })

  afterEach(() => {
    shutdownAnalytics()
    vi.unstubAllEnvs()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('identifies the user by the lowercase hex SHA-256 of the user UUID, the id the server uses', async () => {
    await bootAnalytics(USER_ID, true)

    expect(posthog.identify).toHaveBeenCalledTimes(1)
    expect(posthog.identify).toHaveBeenCalledWith(DISTINCT_ID)
  })

  it('starts PostHog and identifies in one step, so no event goes out anonymously', async () => {
    const booting = bootAnalytics(USER_ID, true)
    // The id is still being hashed: nothing has started and a tracked event is dropped.
    track({ name: 'page_view', props: { path: '/early' } })
    expect(posthog.init).not.toHaveBeenCalled()
    expect(posthog.capture).not.toHaveBeenCalled()

    await booting

    expect(posthog.init).toHaveBeenCalledTimes(1)
    expect(vi.mocked(posthog.init).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(posthog.identify).mock.invocationCallOrder[0]
    )
    track({ name: 'page_view', props: { path: '/late' } })
    expect(posthog.capture).toHaveBeenCalledTimes(1)
    expect(posthog.capture).toHaveBeenCalledWith('page_view', { path: '/late' })
  })

  it('does not start PostHog when consent is revoked while the id is being hashed', async () => {
    const booting = bootAnalytics(USER_ID, true)
    shutdownAnalytics()
    await booting

    expect(posthog.init).not.toHaveBeenCalled()
    expect(posthog.identify).not.toHaveBeenCalled()
  })

  it('starts PostHog once when a second boot overlaps the first', async () => {
    await Promise.all([bootAnalytics(USER_ID, true), bootAnalytics(USER_ID, true)])

    expect(posthog.init).toHaveBeenCalledTimes(1)
    expect(posthog.identify).toHaveBeenCalledTimes(1)
    expect(posthog.identify).toHaveBeenCalledWith(DISTINCT_ID)
  })

  it('does nothing without consent', async () => {
    await bootAnalytics(USER_ID, false)

    expect(posthog.init).not.toHaveBeenCalled()
    expect(posthog.identify).not.toHaveBeenCalled()
  })

  it('stays off, without throwing, when the browser has no WebCrypto', async () => {
    vi.stubGlobal('crypto', {})
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {})

    await expect(bootAnalytics(USER_ID, true)).resolves.toBeUndefined()

    expect(posthog.init).not.toHaveBeenCalled()
    expect(posthog.identify).not.toHaveBeenCalled()
    expect(logged).toHaveBeenCalledTimes(1)
    expect(JSON.stringify(logged.mock.calls)).not.toContain(USER_ID)
  })
})
