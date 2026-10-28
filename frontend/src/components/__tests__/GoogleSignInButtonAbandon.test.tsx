// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Regression tests for a Google popup whose promise never settles (COOP hides popup.closed)
// ABOUTME: Back on the tab with no result, the button resets quietly; a late success still signs in

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, fireEvent, act, cleanup } from '@testing-library/react'
import { GoogleSignInButton } from '../GoogleSignInButton'
import { POPUP_RETURN_GRACE_MS } from '../../hooks/usePopupReturnWatch'

// Mocked: the popup is the browser and Google, which jsdom cannot open, and the
// backend exchange is useAuth's. These tests drive only when each one settles;
// e2e/sign-in-landing.spec.ts runs the button with the real loginWithFirebase.
const { loginWithFirebase } = vi.hoisted(() => ({ loginWithFirebase: vi.fn() }))

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ loginWithFirebase }),
}))
vi.mock('../../firebase/firebase', () => ({
  signInWithGoogle: vi.fn(),
}))

interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason: unknown) => void
}

/** A popup result the test settles by hand — or never, which is the bug. */
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

/** Lets the click handler's dynamic import and awaits run. */
async function flush() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0)
  })
}

async function clickGoogle(onError = vi.fn(), onStart = vi.fn()) {
  render(<GoogleSignInButton dividerSurface="bg-surface" onStart={onStart} onError={onError} />)
  fireEvent.click(screen.getByRole('button', { name: 'Continue with Google' }))
  await flush()
  expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled()
  return { onError, onStart }
}

/** The athlete comes back to our tab: the popup closed, or they left it behind. */
async function returnToTab() {
  await act(async () => {
    window.dispatchEvent(new Event('focus'))
    document.dispatchEvent(new Event('visibilitychange'))
  })
}

async function waitOutGrace() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POPUP_RETURN_GRACE_MS + 1)
  })
}

describe('GoogleSignInButton — a popup that never answers', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.useFakeTimers()
    loginWithFirebase.mockResolvedValue(undefined)
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('resets the button quietly once the athlete is back and no result came', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    vi.mocked(signInWithGoogle).mockReturnValue(new Promise<string | null>(() => {}))

    const { onError } = await clickGoogle()
    await returnToTab()
    await waitOutGrace()

    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()
    expect(onError).not.toHaveBeenCalled()
  })

  it('keeps spinning inside the grace period, while the result may still be on its way', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    vi.mocked(signInWithGoogle).mockReturnValue(new Promise<string | null>(() => {}))

    await clickGoogle()
    await returnToTab()
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POPUP_RETURN_GRACE_MS - 1)
    })

    expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled()
  })

  it('does not reset while the athlete is still in the popup', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    vi.mocked(signInWithGoogle).mockReturnValue(new Promise<string | null>(() => {}))

    await clickGoogle()
    await returnToTab()
    // A glance back at our tab, then back to the popup: the timer is cancelled.
    await act(async () => {
      window.dispatchEvent(new Event('blur'))
    })
    await waitOutGrace()

    expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled()
  })

  it('still signs the athlete in when the popup answers after the reset', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    const popup = deferred<string | null>()
    vi.mocked(signInWithGoogle).mockReturnValue(popup.promise)

    const { onError } = await clickGoogle()
    await returnToTab()
    await waitOutGrace()
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()

    popup.resolve('late-id-token')
    await flush()

    expect(loginWithFirebase).toHaveBeenCalledWith('late-id-token')
    expect(onError).not.toHaveBeenCalled()
  })

  it('spins again and reports a sign-in failure after a late popup success', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    const popup = deferred<string | null>()
    const login = deferred<void>()
    vi.mocked(signInWithGoogle).mockReturnValue(popup.promise)
    loginWithFirebase.mockReturnValue(login.promise)

    const { onError } = await clickGoogle()
    await returnToTab()
    await waitOutGrace()

    // Google completed: the athlete did not walk away from this step.
    popup.resolve('late-id-token')
    await flush()
    expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled()

    login.reject(new Error('backend refused the token'))
    await flush()

    expect(onError).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()
  })

  it('shows no banner when the popup fails after the reset', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    const popup = deferred<string | null>()
    vi.mocked(signInWithGoogle).mockReturnValue(popup.promise)

    const { onError } = await clickGoogle()
    await returnToTab()
    await waitOutGrace()

    popup.reject(Object.assign(new Error('Firebase: Error (auth/internal-error).'), { code: 'auth/internal-error' }))
    await flush()

    expect(onError).not.toHaveBeenCalled()
    expect(loginWithFirebase).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()
  })

  it('still reports a failure that arrives before any abandon', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    const popup = deferred<string | null>()
    vi.mocked(signInWithGoogle).mockReturnValue(popup.promise)

    const { onError } = await clickGoogle()
    await returnToTab()
    popup.reject(Object.assign(new Error('Firebase: Error (auth/network-request-failed).'), {
      code: 'auth/network-request-failed',
    }))
    await flush()
    // Settled: the watch is gone, so outlasting the grace changes nothing.
    await waitOutGrace()

    expect(onError).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()
  })

  it("lets no answer from an abandoned attempt touch a newer click's spinner", async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    const first = deferred<string | null>()
    const second = deferred<string | null>()
    vi.mocked(signInWithGoogle).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)

    const { onError } = await clickGoogle()
    await returnToTab()
    await waitOutGrace()
    fireEvent.click(screen.getByRole('button', { name: 'Continue with Google' }))
    await flush()
    expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled()

    // The first popup answers late: its sign-in is honoured, but the button
    // belongs to the second attempt, which is still waiting.
    first.resolve('first-id-token')
    await flush()
    expect(loginWithFirebase).toHaveBeenCalledWith('first-id-token')
    expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled()

    second.resolve('second-id-token')
    await flush()
    expect(loginWithFirebase).toHaveBeenLastCalledWith('second-id-token')
    expect(onError).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()
  })

  it('keeps the spinner for the redirect fallback even when focus returns', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    vi.mocked(signInWithGoogle).mockResolvedValue(null)

    await clickGoogle()
    await returnToTab()
    await waitOutGrace()

    expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled()
    expect(loginWithFirebase).not.toHaveBeenCalled()
  })

  it('stops watching on unmount', async () => {
    const { signInWithGoogle } = await import('../../firebase/firebase')
    vi.mocked(signInWithGoogle).mockReturnValue(new Promise<string | null>(() => {}))
    const removeSpy = vi.spyOn(window, 'removeEventListener')

    await clickGoogle()
    await returnToTab()
    removeSpy.mockClear()
    cleanup()

    expect(removeSpy).toHaveBeenCalledWith('focus', expect.any(Function))
    expect(vi.getTimerCount()).toBe(0)
    removeSpy.mockRestore()
  })
})
