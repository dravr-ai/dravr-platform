// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the PostHog distinct id the web app identifies a user with, and that nothing is sent before it is set
// ABOUTME: The server's hash_id (carnet#848); also the CSP-safe init, $pageview kept across boot, re-opt-in, and address scrubbing

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import posthog from 'posthog-js'
import { bootAnalytics, scrubEventAddresses, shutdownAnalytics, track, withoutQuery } from '../analytics'

vi.mock('posthog-js', () => ({
  default: {
    __loaded: false,
    init: vi.fn(),
    identify: vi.fn(),
    capture: vi.fn(),
    reset: vi.fn(),
    opt_out_capturing: vi.fn(),
    opt_in_capturing: vi.fn(),
    has_opted_out_capturing: vi.fn(() => false),
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
    track({ name: 'feature_engaged', props: { feature: 'early' } })
    expect(posthog.init).not.toHaveBeenCalled()
    expect(posthog.capture).not.toHaveBeenCalled()

    await booting

    expect(posthog.init).toHaveBeenCalledTimes(1)
    expect(vi.mocked(posthog.init).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(posthog.identify).mock.invocationCallOrder[0]
    )
    expect(posthog.capture).not.toHaveBeenCalled()
    track({ name: 'feature_engaged', props: { feature: 'late' } })
    expect(posthog.capture).toHaveBeenCalledTimes(1)
    expect(posthog.capture).toHaveBeenCalledWith('feature_engaged', { feature: 'late' })
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

describe('web analytics init', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.stubEnv('VITE_POSTHOG_API_KEY', 'phc_test_key')
  })

  afterEach(() => {
    shutdownAnalytics()
    vi.unstubAllEnvs()
    vi.mocked(posthog.has_opted_out_capturing).mockReturnValue(false)
    ;(posthog as { __loaded: boolean }).__loaded = false
  })

  it('never lets the library load a script from outside the origin, which the CSP refuses', async () => {
    await bootAnalytics(USER_ID, true)

    expect(posthog.init).toHaveBeenCalledWith(
      'phc_test_key',
      expect.objectContaining({
        disable_external_dependency_loading: true,
        disable_surveys: true,
        disable_session_recording: true,
        autocapture: false,
        capture_pageview: false,
      })
    )
  })

  it('opts a browser a sign-out or revoke opted out back in before identifying, without an $opt_in event', async () => {
    vi.mocked(posthog.has_opted_out_capturing).mockReturnValue(true)

    await bootAnalytics(USER_ID, true)

    expect(posthog.opt_in_capturing).toHaveBeenCalledWith({ captureEventName: false })
    expect(vi.mocked(posthog.opt_in_capturing).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(posthog.identify).mock.invocationCallOrder[0]
    )
  })

  it('leaves a browser that never opted out as it is', async () => {
    await bootAnalytics(USER_ID, true)

    expect(posthog.opt_in_capturing).not.toHaveBeenCalled()
  })

  it('reuses the page’s instance on a boot after a shutdown instead of initializing it again', async () => {
    await bootAnalytics(USER_ID, true)
    shutdownAnalytics()
    expect(posthog.opt_out_capturing).toHaveBeenCalledTimes(1)
    expect(posthog.reset).toHaveBeenCalledTimes(1)
    ;(posthog as { __loaded: boolean }).__loaded = true
    vi.mocked(posthog.has_opted_out_capturing).mockReturnValue(true)

    await bootAnalytics(USER_ID, true)

    expect(posthog.init).toHaveBeenCalledTimes(1)
    expect(posthog.opt_in_capturing).toHaveBeenCalledTimes(1)
    expect(posthog.identify).toHaveBeenCalledTimes(2)
  })
})

describe('web analytics page views', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.stubEnv('VITE_POSTHOG_API_KEY', 'phc_test_key')
    window.location.hash = '#home'
  })

  afterEach(() => {
    shutdownAnalytics()
    vi.unstubAllEnvs()
    window.location.hash = ''
  })

  it('sends PostHog’s own $pageview with the full address and the route path', async () => {
    await bootAnalytics(USER_ID, true)
    window.location.hash = '#settings/privacy'

    track({ name: '$pageview', props: { path: '/settings/privacy' } })

    expect(posthog.capture).toHaveBeenCalledTimes(1)
    expect(posthog.capture).toHaveBeenCalledWith('$pageview', {
      path: '/settings/privacy',
      $current_url: window.location.href,
    })
    expect(window.location.href).toContain('#settings/privacy')
  })

  it('keeps the page reported while the boot is under way and sends it once identified', async () => {
    const booting = bootAnalytics(USER_ID, true)
    track({ name: '$pageview', props: { path: '/home' } })
    const firstUrl = window.location.href
    window.location.hash = '#chat'
    track({ name: '$pageview', props: { path: '/chat' } })
    expect(posthog.capture).not.toHaveBeenCalled()

    await booting

    // Only the latest page is kept: the one the user is on.
    expect(posthog.capture).toHaveBeenCalledTimes(1)
    expect(posthog.capture).toHaveBeenCalledWith('$pageview', {
      path: '/chat',
      $current_url: window.location.href,
    })
    expect(window.location.href).not.toBe(firstUrl)
    expect(vi.mocked(posthog.identify).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(posthog.capture).mock.invocationCallOrder[0]
    )
  })

  it('forgets the held page on a shutdown, so a later boot does not send it', async () => {
    track({ name: '$pageview', props: { path: '/home' } })
    shutdownAnalytics()

    await bootAnalytics(USER_ID, true)

    expect(posthog.capture).not.toHaveBeenCalled()
  })

  it('sends nothing for a held page while consent is off', async () => {
    track({ name: '$pageview', props: { path: '/home' } })

    await bootAnalytics(USER_ID, false)

    expect(posthog.init).not.toHaveBeenCalled()
    expect(posthog.capture).not.toHaveBeenCalled()
  })
})

describe('web analytics address scrubbing', () => {
  it('drops query strings before and after the hash route', () => {
    expect(withoutQuery('https://app.dravr.ai/verify?token=secret')).toBe('https://app.dravr.ai/verify')
    expect(withoutQuery('https://app.dravr.ai/?utm_source=x#/settings?tab=privacy&code=abc')).toBe(
      'https://app.dravr.ai/#/settings',
    )
    expect(withoutQuery('https://app.dravr.ai/#/chat')).toBe('https://app.dravr.ai/#/chat')
    expect(withoutQuery('https://app.dravr.ai/')).toBe('https://app.dravr.ai/')
  })

  it('drops a fragment of parameters, where the mailed links carry their single-use token', () => {
    expect(withoutQuery('https://app.dravr.ai/verify-email#token=sel.verifier')).toBe(
      'https://app.dravr.ai/verify-email',
    )
    expect(withoutQuery('https://app.dravr.ai/accept-invite?x=1#token=sel.verifier')).toBe(
      'https://app.dravr.ai/accept-invite',
    )
    expect(withoutQuery('https://app.dravr.ai/#chat/3f2a')).toBe('https://app.dravr.ai/#chat/3f2a')
  })

  it('scrubs every address property on the event and the person', () => {
    const scrubbed = scrubEventAddresses({
      uuid: 'u',
      event: '$pageview',
      properties: {
        $current_url: 'https://app.dravr.ai/reset?token=t',
        $session_entry_url: 'https://app.dravr.ai/verify-email#token=t',
        $referrer: 'https://mail.example.com/?q=me',
        path: '/reset',
      },
      $set: { $current_url: 'https://app.dravr.ai/x?y=1' },
      $set_once: { $initial_current_url: 'https://app.dravr.ai/?state=s' },
    } as Parameters<typeof scrubEventAddresses>[0])
    expect(scrubbed?.properties.$current_url).toBe('https://app.dravr.ai/reset')
    expect(scrubbed?.properties.$session_entry_url).toBe('https://app.dravr.ai/verify-email')
    expect(scrubbed?.properties.$referrer).toBe('https://mail.example.com/')
    expect(scrubbed?.properties.path).toBe('/reset')
    expect(scrubbed?.$set?.$current_url).toBe('https://app.dravr.ai/x')
    expect(scrubbed?.$set_once?.$initial_current_url).toBe('https://app.dravr.ai/')
  })

  it('passes a dropped event through', () => {
    expect(scrubEventAddresses(null)).toBeNull()
  })
})
