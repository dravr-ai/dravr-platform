// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { StrictMode } from 'react'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { WEB_AUTH_FAILURE_EVENT } from '@pierre/api-client/adapters/web'
import { AuthProvider } from '../AuthContext'
import { useAuth } from '../../hooks/useAuth'
import { PENDING_SIGN_IN_KEY, type PendingSignIn } from '../../utils/signIn'

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

// Mock the API service
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

// Test component that uses the auth context
function TestComponent() {
  const { user, isAuthenticated, startSignIn, signInFailure, logout, loading } = useAuth()

  return (
    <div>
      <div data-testid="loading">{loading ? 'Loading' : 'Not Loading'}</div>
      <div data-testid="authenticated">{isAuthenticated ? 'Authenticated' : 'Not Authenticated'}</div>
      {user && <div data-testid="user-email">{user.email}</div>}
      {signInFailure && (
        <div data-testid="sign-in-failure">
          {signInFailure.kind === 'callback' ? signInFailure.error : 'exchange'}
        </div>
      )}
      <button onClick={() => void startSignIn()} data-testid="login-btn">
        Login
      </button>
      <button onClick={logout} data-testid="logout-btn">
        Logout
      </button>
    </div>
  )
}

function renderWithAuth() {
  return render(
    <AuthProvider>
      <TestComponent />
    </AuthProvider>
  )
}

describe('AuthContext', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
  })

  it('should render in unauthenticated state initially', () => {
    renderWithAuth()

    expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
    expect(screen.getByTestId('loading')).toHaveTextContent('Not Loading')
    expect(screen.queryByTestId('user-email')).not.toBeInTheDocument()
  })

  it('should restore session from cookie when user exists in localStorage', async () => {
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }
    localStorage.setItem('pierre_user', JSON.stringify(mockUser))

    const { authApi } = await import('../../services/api')

    // Mock session restore succeeding
    vi.mocked(authApi.getSession).mockResolvedValue({
      user: mockUser,
      access_token: 'fresh-jwt-token',
      csrf_token: 'fresh-csrf-token',
    })

    renderWithAuth()

    await waitFor(() => {
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Authenticated')
      expect(screen.getByTestId('user-email')).toHaveTextContent('test@example.com')
    })

    expect(authApi.getSession).toHaveBeenCalled()
    expect(mockAuthStorage.setCsrfToken).toHaveBeenCalledWith('fresh-csrf-token')
  })

  it('should clear auth state when session restore fails', async () => {
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }
    localStorage.setItem('pierre_user', JSON.stringify(mockUser))

    const { authApi } = await import('../../services/api')

    // Mock session restore failing (expired cookie)
    vi.mocked(authApi.getSession).mockRejectedValue(new Error('401 Unauthorized'))

    renderWithAuth()

    // Initially shows cached user
    expect(screen.getByTestId('authenticated')).toHaveTextContent('Authenticated')

    // After session restore fails, should clear auth state
    await waitFor(() => {
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
    })

    expect(localStorage.getItem('pierre_user')).toBeNull()
  })

  it('keeps the athlete signed in when session restore is rate limited', async () => {
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }
    localStorage.setItem('pierre_user', JSON.stringify(mockUser))

    const { authApi } = await import('../../services/api')

    // A spent request budget: the cookie is valid, the server says "wait".
    vi.mocked(authApi.getSession).mockRejectedValue({
      response: {
        status: 429,
        data: {
          code: 'RateLimitExceeded',
          message: 'Rate limit exceeded: 10000/10000 requests, retry after 3600s',
          details: { limit_type: 'requests', current: 10000, limit: 10000, retry_after_secs: 3600 },
        },
      },
    })

    renderWithAuth()

    await waitFor(() => {
      expect(screen.getByTestId('loading')).toHaveTextContent('Not Loading')
    })
    expect(authApi.getSession).toHaveBeenCalled()
    expect(screen.getByTestId('authenticated')).toHaveTextContent('Authenticated')
    expect(screen.getByTestId('user-email')).toHaveTextContent('test@example.com')
    expect(localStorage.getItem('pierre_user')).toBe(JSON.stringify(mockUser))
  })

  it('still signs out when session restore is refused as unauthorized', async () => {
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }
    localStorage.setItem('pierre_user', JSON.stringify(mockUser))

    const { authApi } = await import('../../services/api')
    vi.mocked(authApi.getSession).mockRejectedValue({
      response: { status: 401, data: { code: 'AuthInvalid', message: 'Authentication failed' } },
    })

    renderWithAuth()

    await waitFor(() => {
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
    })
    expect(localStorage.getItem('pierre_user')).toBeNull()
  })

  it('should logout successfully', async () => {
    const user = userEvent.setup()
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }

    localStorage.setItem('pierre_user', JSON.stringify(mockUser))

    const { authApi } = await import('../../services/api')

    // Mock session restore for initial load
    vi.mocked(authApi.getSession).mockResolvedValue({
      user: mockUser,
      access_token: 'fresh-jwt',
      csrf_token: 'fresh-csrf',
    })

    renderWithAuth()

    // Wait for initial authentication via session restore
    await waitFor(() => {
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Authenticated')
    })

    // Click logout button
    await user.click(screen.getByTestId('logout-btn'))

    // Should be unauthenticated after logout
    await waitFor(() => {
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
    })

    // authApi.logout() handles authStorage.clear() internally (in its finally block),
    // so we only verify the logout API was called — the mock replaces the real implementation
    expect(authApi.logout).toHaveBeenCalled()
    expect(screen.queryByTestId('user-email')).not.toBeInTheDocument()
  })

  it('should sign out when the transport reports an auth failure', async () => {
    // The other half of the interceptor's recovery path: it clears storage and
    // fires this event, and nothing signs the session out unless somebody is
    // listening. Nothing was asserting that anybody was.
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }
    localStorage.setItem('pierre_user', JSON.stringify(mockUser))

    const { authApi } = await import('../../services/api')
    vi.mocked(authApi.getSession).mockResolvedValue({
      user: mockUser,
      access_token: 'fresh-jwt',
      csrf_token: 'fresh-csrf',
    })

    renderWithAuth()

    await waitFor(() => {
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Authenticated')
    })

    act(() => {
      window.dispatchEvent(new CustomEvent(WEB_AUTH_FAILURE_EVENT))
    })

    await waitFor(() => {
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
    })

    expect(authApi.logout).toHaveBeenCalled()
    expect(screen.queryByTestId('user-email')).not.toBeInTheDocument()
    await waitFor(() => {
      expect(localStorage.getItem('pierre_user')).toBeNull()
    })
  })

  it('should show loading state during session restore', async () => {
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }
    localStorage.setItem('pierre_user', JSON.stringify(mockUser))

    const { authApi } = await import('../../services/api')

    // Make session restore hang to test loading state
    vi.mocked(authApi.getSession).mockImplementation(() => new Promise(() => {}))

    renderWithAuth()

    // Should show loading state while session restores
    expect(screen.getByTestId('loading')).toHaveTextContent('Loading')
  })

  describe('hosted sign-in (carnet#787)', () => {
    const ORIGIN = window.location.origin
    const REDIRECT_URI = `${ORIGIN}/auth/callback`
    const mockUser = { id: '1', email: 'test@example.com', display_name: 'Test User' }

    function storePending(pending: Partial<PendingSignIn> = {}) {
      sessionStorage.setItem(
        PENDING_SIGN_IN_KEY,
        JSON.stringify({
          codeVerifier: 'verifier-1',
          state: 'state-1',
          redirectUri: REDIRECT_URI,
          deepLink: null,
          ...pending,
        }),
      )
    }

    /** Land on the hosted sign-in's return leg with `query`. */
    function landOnCallback(query: string) {
      window.history.replaceState(null, '', `/auth/callback?${query}`)
    }

    beforeEach(() => {
      sessionStorage.clear()
      window.history.replaceState(null, '', '/')
    })

    afterEach(() => {
      vi.unstubAllGlobals()
      window.history.replaceState(null, '', '/')
    })

    it('opens the hosted sign-in and keeps the verifier, state and followed link for the return leg', async () => {
      const user = userEvent.setup()
      const { authApi } = await import('../../services/api')
      vi.mocked(authApi.beginSignIn).mockResolvedValue({
        authorizeUrl: `${ORIGIN}/oauth2/authorize?state=state-1`,
        codeVerifier: 'verifier-1',
        state: 'state-1',
        redirectUri: REDIRECT_URI,
      })
      const assign = vi.fn()
      vi.stubGlobal('location', { ...window.location, origin: ORIGIN, pathname: '/', assign })

      renderWithAuth()
      await user.click(screen.getByTestId('login-btn'))

      await waitFor(() => expect(assign).toHaveBeenCalledWith(`${ORIGIN}/oauth2/authorize?state=state-1`))
      const [redirectUri, crypto] = vi.mocked(authApi.beginSignIn).mock.calls[0]
      expect(redirectUri).toBe(REDIRECT_URI)
      expect(typeof crypto.randomBytes).toBe('function')
      expect(typeof crypto.sha256).toBe('function')
      expect(JSON.parse(sessionStorage.getItem(PENDING_SIGN_IN_KEY) ?? 'null')).toEqual({
        codeVerifier: 'verifier-1',
        state: 'state-1',
        redirectUri: REDIRECT_URI,
        deepLink: null,
      })
    })

    it('redeems the code, establishes the session and restores the followed link', async () => {
      const { authApi } = await import('../../services/api')
      vi.mocked(authApi.completeSignIn).mockResolvedValue({
        user: mockUser,
        access_token: 'jwt-token-value',
        csrf_token: 'csrf-test-token',
        expires_at: new Date(Date.now() + 86400000).toISOString(),
      } as never)
      storePending({ deepLink: 'chat/conv-1' })
      landOnCallback('code=code-success&state=state-1')

      renderWithAuth()
      expect(screen.getByTestId('loading')).toHaveTextContent('Loading')

      await waitFor(() => {
        expect(screen.getByTestId('authenticated')).toHaveTextContent('Authenticated')
      })
      expect(authApi.completeSignIn).toHaveBeenCalledWith({
        code: 'code-success',
        codeVerifier: 'verifier-1',
        redirectUri: REDIRECT_URI,
      })
      expect(screen.getByTestId('user-email')).toHaveTextContent('test@example.com')
      expect(mockAuthStorage.setCsrfToken).toHaveBeenCalledWith('csrf-test-token')
      // The spent code leaves the address bar; the followed link comes back.
      expect(window.location.pathname).toBe('/')
      expect(window.location.search).toBe('')
      expect(window.location.hash).toBe('#chat/conv-1')
      // A verifier redeems one code: it is gone once read.
      expect(sessionStorage.getItem(PENDING_SIGN_IN_KEY)).toBeNull()
      // JWT stays in memory (WebSocket), never in localStorage.
      expect(localStorage.getItem('pierre_auth_token')).toBeNull()
      expect(localStorage.getItem('pierre_user')).not.toBeNull()
    })

    it('refuses a callback whose state this tab did not send, without redeeming its code', async () => {
      const { authApi } = await import('../../services/api')
      storePending({ state: 'state-1' })
      landOnCallback('code=code-forged&state=someone-elses')

      renderWithAuth()

      await waitFor(() => {
        expect(screen.getByTestId('sign-in-failure')).toHaveTextContent('state_mismatch')
      })
      expect(authApi.completeSignIn).not.toHaveBeenCalled()
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
      expect(window.location.pathname).toBe('/')
      expect(sessionStorage.getItem(PENDING_SIGN_IN_KEY)).toBeNull()
    })

    it('refuses a callback when no sign-in was started in this tab', async () => {
      const { authApi } = await import('../../services/api')
      landOnCallback('code=code-orphan&state=state-1')

      renderWithAuth()

      await waitFor(() => {
        expect(screen.getByTestId('sign-in-failure')).toHaveTextContent('state_mismatch')
      })
      expect(authApi.completeSignIn).not.toHaveBeenCalled()
    })

    it('reports the refusal the server sent back (a suspended account)', async () => {
      const { authApi } = await import('../../services/api')
      storePending()
      landOnCallback('error=access_denied&error_description=Your+Dravr+account+is+suspended&state=state-1')

      renderWithAuth()

      await waitFor(() => {
        expect(screen.getByTestId('sign-in-failure')).toHaveTextContent('access_denied')
      })
      expect(authApi.completeSignIn).not.toHaveBeenCalled()
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
      expect(window.location.pathname).toBe('/')
    })

    it('reports a code that would not redeem', async () => {
      const { authApi } = await import('../../services/api')
      vi.mocked(authApi.completeSignIn).mockRejectedValue({
        response: { status: 400, data: { error: 'invalid_grant' } },
      })
      storePending()
      landOnCallback('code=code-spent&state=state-1')

      renderWithAuth()

      await waitFor(() => {
        expect(screen.getByTestId('sign-in-failure')).toHaveTextContent('exchange')
      })
      expect(screen.getByTestId('authenticated')).toHaveTextContent('Not Authenticated')
    })

    it('does not sign out on an auth-failure event while the return leg is being redeemed', async () => {
      const { authApi } = await import('../../services/api')
      vi.mocked(authApi.completeSignIn).mockImplementation(() => new Promise(() => {}))
      storePending()
      landOnCallback('code=code-hanging&state=state-1')

      renderWithAuth()

      act(() => {
        window.dispatchEvent(new CustomEvent(WEB_AUTH_FAILURE_EVENT))
      })

      expect(authApi.logout).not.toHaveBeenCalled()
      expect(screen.getByTestId('loading')).toHaveTextContent('Loading')
    })

    it('redeems the code once under StrictMode double effects', async () => {
      const { authApi } = await import('../../services/api')
      vi.mocked(authApi.completeSignIn).mockResolvedValue({
        user: mockUser,
        access_token: 'jwt',
        csrf_token: 'csrf',
        expires_at: new Date(Date.now() + 86400000).toISOString(),
      } as never)
      storePending()
      landOnCallback('code=code-strict&state=state-1')

      render(
        <StrictMode>
          <AuthProvider>
            <TestComponent />
          </AuthProvider>
        </StrictMode>,
      )

      await waitFor(() => {
        expect(screen.getByTestId('authenticated')).toHaveTextContent('Authenticated')
      })
      expect(authApi.completeSignIn).toHaveBeenCalledTimes(1)
      expect(screen.queryByTestId('sign-in-failure')).not.toBeInTheDocument()
    })
  })
})
