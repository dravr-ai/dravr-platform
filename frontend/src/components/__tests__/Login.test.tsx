// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, waitFor, act } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { PENDING_SIGN_IN_KEY } from '../../utils/signIn'
import Login from '../Login'
import { AuthProvider } from '../../contexts/AuthContext'
import { ThemeProvider } from '../../hooks/useTheme'

// vi.hoisted runs before vi.mock hoisting, so these variables are available in the factory
const { mockAuthStorage } = vi.hoisted(() => ({
  mockAuthStorage: {
    setCsrfToken: vi.fn().mockResolvedValue(undefined),
    getCsrfToken: vi.fn().mockResolvedValue(null),
    setUser: vi.fn().mockResolvedValue(undefined),
    getUser: vi.fn().mockResolvedValue(null),
    clear: vi.fn().mockResolvedValue(undefined),
    getToken: vi.fn().mockResolvedValue(null),
    setToken: vi.fn().mockResolvedValue(undefined),
    removeToken: vi.fn().mockResolvedValue(undefined),
    getRefreshToken: vi.fn().mockResolvedValue(null),
    setRefreshToken: vi.fn().mockResolvedValue(undefined),
  },
}))

// Mock the API service - AuthContext uses authApi, pierreApi, adminApi
vi.mock('../../services/api', () => ({
  authApi: {
    beginSignIn: vi.fn(),
    completeSignIn: vi.fn(),
    logout: vi.fn().mockResolvedValue(undefined),
    getSession: vi.fn(),
  },
  adminApi: {
    endImpersonation: vi.fn(),
  },
  pierreApi: {
    adapter: {
      authStorage: mockAuthStorage,
    },
  },
}))

// No Google redirect is in flight: the Firebase SDK's own redirect check has
// no network in jsdom and would otherwise put its failure in the banner.
vi.mock('../../firebase/firebase', () => ({
  getGoogleRedirectResult: vi.fn().mockResolvedValue(null),
  signInWithGoogle: vi.fn(),
}))

async function renderLogin() {
  let result;
  await act(async () => {
    result = render(
      <ThemeProvider>
        <AuthProvider>
          <Login onNavigateToForgotPassword={() => {}} onNavigateToRegister={() => {}} />
        </AuthProvider>
      </ThemeProvider>
    );
  });
  return result;
}

/** Land on the hosted sign-in's return leg with `query`, a sign-in pending in this tab. */
function landOnCallback(query: string) {
  sessionStorage.setItem(
    PENDING_SIGN_IN_KEY,
    JSON.stringify({
      codeVerifier: 'verifier-1',
      state: 'state-1',
      redirectUri: `${window.location.origin}/auth/callback`,
      deepLink: null,
    }),
  )
  window.history.replaceState(null, '', `/auth/callback?${query}`)
}

/** Pin navigator.onLine for one test; jsdom reports true by default. */
function setOnline(value: boolean) {
  Object.defineProperty(window.navigator, 'onLine', {
    configurable: true,
    get: () => value,
  })
}

describe('Login Component', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    sessionStorage.clear()
    localStorage.clear()
    window.history.replaceState(null, '', '/')
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    setOnline(true)
    window.history.replaceState(null, '', '/')
  })

  it('offers the hosted sign-in, and never asks for a password itself', async () => {
    await renderLogin()

    expect(screen.getByRole('heading', { name: /sign in/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Sign in with email' })).toBeInTheDocument()
    // carnet#787: the password is typed on the server's page, never in the app.
    expect(screen.queryByLabelText(/email address/i)).not.toBeInTheDocument()
    expect(screen.queryByLabelText(/^password$/i)).not.toBeInTheDocument()
    expect(document.querySelector('input[type="password"]')).toBeNull()
    expect(screen.getByRole('button', { name: /forgot password/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /create one/i })).toBeInTheDocument()
  })

  it('opens the hosted sign-in when the button is pressed, and keeps the spinner up while leaving', async () => {
    const user = userEvent.setup()
    const { authApi } = await import('../../services/api')
    const authorizeUrl = `${window.location.origin}/oauth2/authorize?state=state-1`
    vi.mocked(authApi.beginSignIn).mockResolvedValue({
      authorizeUrl,
      codeVerifier: 'verifier-1',
      state: 'state-1',
      redirectUri: `${window.location.origin}/auth/callback`,
    })
    const assign = vi.fn()
    vi.stubGlobal('location', { ...window.location, origin: window.location.origin, pathname: '/', assign })

    await renderLogin()
    await user.click(screen.getByRole('button', { name: 'Sign in with email' }))

    await waitFor(() => expect(assign).toHaveBeenCalledWith(authorizeUrl))
    expect(screen.getByRole('button', { name: /signing in/i })).toBeDisabled()
    expect(sessionStorage.getItem(PENDING_SIGN_IN_KEY)).not.toBeNull()
  })

  it('says the sign-in failed when it could not even be started', async () => {
    const user = userEvent.setup()
    const { authApi } = await import('../../services/api')
    // e.g. WebCrypto absent on an insecure origin
    vi.mocked(authApi.beginSignIn).mockRejectedValue(new TypeError('crypto.subtle is undefined'))

    await renderLogin()
    await user.click(screen.getByRole('button', { name: 'Sign in with email' }))

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Sign-in failed')
    })
    expect(screen.getByRole('button', { name: 'Sign in with email' })).not.toBeDisabled()
  })

  it('reads a refused sign-in for a suspended account as the suspension', async () => {
    landOnCallback('error=access_denied&error_description=Your+Dravr+account+is+suspended&state=state-1')

    await renderLogin()

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(
        'Your account has been suspended. Contact an administrator for help.',
      )
    })
    // The server's English description is never shown.
    expect(screen.queryByText(/Your Dravr account is suspended/)).not.toBeInTheDocument()
    expect(window.location.pathname).toBe('/')
  })

  it('refuses a return leg this tab did not start, without blaming a password', async () => {
    const { authApi } = await import('../../services/api')
    landOnCallback('code=forged-code&state=not-mine')

    await renderLogin()

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Sign-in failed')
    })
    expect(authApi.completeSignIn).not.toHaveBeenCalled()
    expect(screen.queryByText('Invalid email or password')).not.toBeInTheDocument()
  })

  it('reads a code that would not redeem as a failed sign-in, never as a wrong password', async () => {
    const { authApi } = await import('../../services/api')
    vi.mocked(authApi.completeSignIn).mockRejectedValue({
      response: { status: 400, data: { error: 'invalid_grant' } },
    })
    landOnCallback('code=spent-code&state=state-1')

    await renderLogin()

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Sign-in failed')
    })
    expect(screen.queryByText('Invalid email or password')).not.toBeInTheDocument()
  })

  it('tells an OFFLINE athlete they are offline when the code exchange never left', async () => {
    const { authApi } = await import('../../services/api')
    // A request that never reached a server: no `response` on the rejection.
    vi.mocked(authApi.completeSignIn).mockRejectedValue(new Error('Network Error'))
    setOnline(false)
    landOnCallback('code=offline-code&state=state-1')

    await renderLogin()

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(
        "You're offline. Check your connection and try again.",
      )
    })
  })

  it('clears the reported failure when a new sign-in starts', async () => {
    const user = userEvent.setup()
    const { authApi } = await import('../../services/api')
    vi.mocked(authApi.beginSignIn).mockImplementation(() => new Promise(() => {}))
    landOnCallback('error=access_denied&state=state-1')

    await renderLogin()
    await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument())

    await user.click(screen.getByRole('button', { name: 'Sign in with email' }))

    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })
})
