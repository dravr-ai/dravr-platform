// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the first-party sign-in each app performs: the authorize URL it opens and the code exchange it sends
// ABOUTME: Authorization code + PKCE (S256) as the named first-party client; never a password grant (carnet#787)

import { describe, it, expect, vi } from 'vitest'
import { createHash } from 'node:crypto'
import type { AxiosInstance } from 'axios'
import { createAuthApi, readSignInCallback } from '@pierre/api-client'
import type { AuthStorage } from '@pierre/api-client'
import { webPkceCrypto } from '../../utils/signIn'

function memoryStorage(): AuthStorage {
  return {
    getToken: vi.fn().mockResolvedValue(null),
    setToken: vi.fn().mockResolvedValue(undefined),
    removeToken: vi.fn().mockResolvedValue(undefined),
    getCsrfToken: vi.fn().mockResolvedValue(null),
    setCsrfToken: vi.fn().mockResolvedValue(undefined),
    getUser: vi.fn().mockResolvedValue(null),
    setUser: vi.fn().mockResolvedValue(undefined),
    getRefreshToken: vi.fn().mockResolvedValue(null),
    setRefreshToken: vi.fn().mockResolvedValue(undefined),
    clear: vi.fn().mockResolvedValue(undefined),
  }
}

const REDIRECT_URI = 'https://app.dravr.test/auth/callback'

/** Redeem a code on `platform` and return the form the token endpoint received. */
async function exchangeForm(platform: 'web' | 'mobile'): Promise<{ url: string; form: URLSearchParams }> {
  const post = vi.fn().mockResolvedValue({ data: {} })
  const axios = { post, defaults: {} } as unknown as AxiosInstance
  await createAuthApi(axios, memoryStorage(), platform).completeSignIn({
    code: 'the-code',
    codeVerifier: 'the-verifier',
    redirectUri: REDIRECT_URI,
  })
  expect(post).toHaveBeenCalledTimes(1)
  return { url: post.mock.calls[0][0] as string, form: new URLSearchParams(post.mock.calls[0][1] as string) }
}

/** RFC 4648 §5 base64url, unpadded — computed independently of the client's encoder. */
function base64Url(bytes: Buffer): string {
  return bytes.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

describe('first-party code exchange', () => {
  it('redeems an authorization code with its PKCE verifier as the web app', async () => {
    const { url, form } = await exchangeForm('web')
    expect(url).toBe('/oauth/token')
    expect(form.get('grant_type')).toBe('authorization_code')
    expect(form.get('client_id')).toBe('dravr-web')
    expect(form.get('code')).toBe('the-code')
    expect(form.get('code_verifier')).toBe('the-verifier')
    expect(form.get('redirect_uri')).toBe(REDIRECT_URI)
    expect(form.get('scope')).toBeNull()
    // The password never travels to the token endpoint any more.
    expect(form.get('password')).toBeNull()
    expect(form.get('username')).toBeNull()
  })

  it('names the mobile app on the phone, which also asks for a refresh token', async () => {
    const { form } = await exchangeForm('mobile')
    expect(form.get('grant_type')).toBe('authorization_code')
    expect(form.get('client_id')).toBe('dravr-mobile')
    expect(form.get('scope')).toBe('offline_access')
  })
})

describe('beginSignIn with the browser WebCrypto', () => {
  it('opens /oauth2/authorize with an S256 challenge of its own verifier', async () => {
    const axios = { post: vi.fn(), defaults: { baseURL: 'https://app.dravr.test/' } } as unknown as AxiosInstance
    const request = await createAuthApi(axios, memoryStorage(), 'web').beginSignIn(REDIRECT_URI, webPkceCrypto)

    const url = new URL(request.authorizeUrl)
    expect(`${url.origin}${url.pathname}`).toBe('https://app.dravr.test/oauth2/authorize')
    const q = url.searchParams
    expect(q.get('response_type')).toBe('code')
    expect(q.get('client_id')).toBe('dravr-web')
    expect(q.get('redirect_uri')).toBe(REDIRECT_URI)
    expect(q.get('code_challenge_method')).toBe('S256')
    expect(q.get('state')).toBe(request.state)
    // OpenID Connect prompt=login: a session the browser still holds for the
    // previous athlete never signs the next one in.
    expect(q.get('prompt')).toBe('login')
    expect(request.redirectUri).toBe(REDIRECT_URI)

    // RFC 7636 §4.1: 43..128 unreserved characters.
    expect(request.codeVerifier).toMatch(/^[A-Za-z0-9\-._~]{43,128}$/)
    // §4.2: challenge = BASE64URL(SHA256(ASCII(verifier))), checked with Node's hash.
    const expected = base64Url(createHash('sha256').update(request.codeVerifier, 'ascii').digest())
    expect(q.get('code_challenge')).toBe(expected)
    // The verifier itself never leaves with the authorization request.
    expect(request.authorizeUrl).not.toContain(request.codeVerifier)
  })

  it('asks the hosted page for the language the app is showing', async () => {
    const axios = { post: vi.fn(), defaults: {} } as unknown as AxiosInstance
    const api = createAuthApi(axios, memoryStorage(), 'web')
    const french = new URL((await api.beginSignIn(REDIRECT_URI, webPkceCrypto, 'fr')).authorizeUrl, 'https://x.test')
    expect(french.searchParams.get('ui_locales')).toBe('fr')
    const unset = new URL((await api.beginSignIn(REDIRECT_URI, webPkceCrypto)).authorizeUrl, 'https://x.test')
    expect(unset.searchParams.has('ui_locales')).toBe(false)
  })

  it('draws a fresh verifier and state for every sign-in', async () => {
    const axios = { post: vi.fn(), defaults: {} } as unknown as AxiosInstance
    const api = createAuthApi(axios, memoryStorage(), 'web')
    const a = await api.beginSignIn(REDIRECT_URI, webPkceCrypto)
    const b = await api.beginSignIn(REDIRECT_URI, webPkceCrypto)
    expect(a.codeVerifier).not.toBe(b.codeVerifier)
    expect(a.state).not.toBe(b.state)
  })
})

describe('readSignInCallback', () => {
  it('refuses a callback whose state is not the one this sign-in sent', () => {
    const params = new URLSearchParams({ code: 'c', state: 'forged' })
    expect(readSignInCallback(params, 'mine')).toEqual({ kind: 'error', error: 'state_mismatch' })
  })

  it('returns the code of a callback carrying this sign-in state', () => {
    const params = new URLSearchParams({ code: 'c', state: 'mine' })
    expect(readSignInCallback(params, 'mine')).toEqual({ kind: 'code', code: 'c' })
  })
})
