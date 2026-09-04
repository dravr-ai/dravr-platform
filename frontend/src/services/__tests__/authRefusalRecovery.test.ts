// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Drives the REAL shared response interceptor over the web adapter, transport stubbed
// ABOUTME: Pins which refusals sign the athlete out and, above all, which one must not

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { AxiosError, AxiosHeaders } from 'axios'
import type { AxiosInstance, AxiosResponse, InternalAxiosRequestConfig } from 'axios'
import { createAxiosClient, droveReauthentication } from '@pierre/api-client'
import { createWebAdapter, WEB_AUTH_FAILURE_EVENT } from '@pierre/api-client/adapters/web'

// Nothing here is mocked except the wire. The sibling api.test.ts replaces
// `axios` and `@pierre/api-client` wholesale, so the interceptor under test
// never runs there — the only thing that can pin this behaviour is the real
// instance with a stubbed transport underneath it.
function refusalTransport(
  status: number,
  headers: Record<string, string>,
  data: unknown,
): (config: InternalAxiosRequestConfig) => Promise<AxiosResponse> {
  return (config) => {
    const response: AxiosResponse = {
      status,
      statusText: '',
      data,
      headers: AxiosHeaders.from(headers),
      config,
    }
    // What axios itself throws for an out-of-range status: the message below is
    // the literal string the athlete was reading off refused admin tabs.
    return Promise.reject(
      new AxiosError(
        `Request failed with status code ${status}`,
        AxiosError.ERR_BAD_REQUEST,
        config,
        null,
        response,
      ),
    )
  }
}

interface Harness {
  client: AxiosInstance
  clear: ReturnType<typeof vi.spyOn>
  authFailures: () => number
  dispose: () => void
}

function harnessFor(status: number, headers: Record<string, string>, data: unknown): Harness {
  const adapter = createWebAdapter({ baseURL: 'http://api.test' })
  // Spying on the adapter's own storage, not a stand-in: `createAxiosClient`
  // destructures this exact object, so the interceptor calls the spy.
  const clear = vi.spyOn(adapter.authStorage, 'clear')
  const client = createAxiosClient(adapter)
  client.defaults.adapter = refusalTransport(status, headers, data)

  let count = 0
  const listener = () => {
    count += 1
  }
  window.addEventListener(WEB_AUTH_FAILURE_EVENT, listener)

  return {
    client,
    clear,
    authFailures: () => count,
    dispose: () => window.removeEventListener(WEB_AUTH_FAILURE_EVENT, listener),
  }
}

const INSUFFICIENT_SCOPE_CHALLENGE =
  'Bearer resource_metadata="https://x/.well-known/oauth-protected-resource", ' +
  'error="insufficient_scope", scope="fitness:write"'

describe('shared response interceptor: which refusals recover by re-authenticating', () => {
  let harness: Harness | null = null

  beforeEach(() => {
    localStorage.clear()
    harness?.dispose()
    harness = null
  })

  it('signs the athlete out on a 401', async () => {
    harness = harnessFor(401, {}, { code: 'AuthRequired', message: 'Authentication required' })

    await expect(harness.client.get('/api/users/me')).rejects.toMatchObject({
      response: { status: 401 },
    })

    expect(harness.clear).toHaveBeenCalledTimes(1)
    expect(harness.authFailures()).toBe(1)
  })

  it('signs the athlete out on a 403 whose challenge says insufficient_scope', async () => {
    harness = harnessFor(
      403,
      { 'www-authenticate': INSUFFICIENT_SCOPE_CHALLENGE },
      { code: 'InsufficientScope', message: 'Scope fitness:write required' },
    )

    await expect(harness.client.post('/api/activities', {})).rejects.toMatchObject({
      response: { status: 403 },
    })

    // Same recovery as the 401: the grant is too narrow, and a new one fixes it.
    expect(harness.clear).toHaveBeenCalledTimes(1)
    expect(harness.authFailures()).toBe(1)
  })

  it('marks a recovered refusal, so an error surface need not re-derive the verdict', async () => {
    // The mark is how the app-wide surface stays silent while the athlete is
    // being sent to sign in. Asserted on both a 401 and the scope 403, because
    // an app that read only the status would toast straight through the 403.
    harness = harnessFor(401, {}, { code: 'AuthRequired', message: 'Authentication required' })
    const unauthenticated = await harness.client
      .get('/api/users/me')
      .then(() => null)
      .catch((err: unknown) => err)
    expect(droveReauthentication(unauthenticated)).toBe(true)

    harness.dispose()
    harness = harnessFor(
      403,
      { 'www-authenticate': INSUFFICIENT_SCOPE_CHALLENGE },
      { code: 'InsufficientScope', message: 'Scope fitness:write required' },
    )
    const narrowGrant = await harness.client
      .post('/api/activities', {})
      .then(() => null)
      .catch((err: unknown) => err)
    expect(droveReauthentication(narrowGrant)).toBe(true)
  })

  it('reads the challenge, not the body, and not a header of another scheme', async () => {
    // Both of these are 403s that LOOK like a scope problem in their prose. The
    // body is sanitised and localised server-side, so prose is not a signal;
    // only the RFC 6750 challenge is, and only under the Bearer scheme.
    for (const headers of [
      {},
      { 'www-authenticate': 'Basic realm="admin"' },
    ]) {
      harness?.dispose()
      harness = harnessFor(403, headers, {
        code: 'PermissionDenied',
        message: 'insufficient_scope: fitness:write required',
      })

      await expect(harness.client.get('/api/admin/users')).rejects.toMatchObject({
        response: { status: 403 },
      })

      expect(harness.clear).not.toHaveBeenCalled()
      expect(harness.authFailures()).toBe(0)
    }
  })

  it('reads a challenge whose quoted metadata URL contains a comma', async () => {
    // `resource_metadata` is a quoted URL and may legally hold a comma, so a
    // parser that split the header on commas would lose the error code and
    // leave the athlete stranded on the one refusal re-auth actually fixes.
    harness = harnessFor(
      403,
      {
        'www-authenticate':
          'Bearer resource_metadata="https://x/.well-known/oauth-protected-resource?a=1,2", ' +
          'error="insufficient_scope", scope="fitness:write profile:read"',
      },
      { code: 'InsufficientScope', message: 'Scope required' },
    )

    await expect(harness.client.post('/api/activities', {})).rejects.toMatchObject({
      response: { status: 403 },
    })

    expect(harness.clear).toHaveBeenCalledTimes(1)
    expect(harness.authFailures()).toBe(1)
  })

  it('leaves the athlete signed in on a role 403, and still rejects', async () => {
    // The point of the whole change. This refusal survives re-authentication —
    // a plain admin who is signed out lands back on the same 403 — so clearing
    // the session over it is a login loop, not a recovery.
    harness = harnessFor(
      403,
      {},
      { code: 'PermissionDenied', message: 'Super-admin privileges required' },
    )

    const refusal = await harness.client
      .get('/api/admin/users')
      .then(() => null)
      .catch((err: unknown) => err)

    expect(refusal).toBeInstanceOf(AxiosError)
    expect((refusal as AxiosError).message).toBe('Request failed with status code 403')
    expect((refusal as AxiosError<{ code: string; message: string }>).response?.data).toEqual({
      code: 'PermissionDenied',
      message: 'Super-admin privileges required',
    })
    expect(harness.clear).not.toHaveBeenCalled()
    expect(harness.authFailures()).toBe(0)
    // Unmarked, which is what lets the app-wide surface say what was refused
    // instead of staying quiet for a sign-out that is not happening.
    expect(droveReauthentication(refusal)).toBe(false)
  })
})
