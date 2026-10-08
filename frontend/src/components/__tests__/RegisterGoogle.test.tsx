// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for "Continue with Google" on the create-account form
// ABOUTME: Signs a newcomer in, puts a failure in the form's banner, stays quiet on a closed popup

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor, act } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { FirebaseLoginResponse } from '@pierre/shared-types'
import Register from '../Register'
import { AuthProvider } from '../../contexts/AuthContext'
import { useAuth } from '../../hooks/useAuth'

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

// The HTTP layer, as in Login.test.tsx: AuthProvider's real loginWithFirebase
// runs and stores the user, only the request is answered here. The real round
// trip is covered by e2e/sign-in-landing.spec.ts.
vi.mock('../../services/api', () => ({
  authApi: {
    register: vi.fn(),
    loginWithFirebase: vi.fn(),
    beginSignIn: vi.fn(),
    completeSignIn: vi.fn(),
    logout: vi.fn().mockResolvedValue(undefined),
    getSession: vi.fn(),
  },
  adminApi: {
    endImpersonation: vi.fn(),
  },
  userApi: {
    setTimezone: vi.fn().mockResolvedValue(undefined),
  },
  pierreApi: {
    adapter: {
      authStorage: mockAuthStorage,
    },
  },
}))

// The test environment has no Firebase env, so the button would not be drawn;
// jsdom cannot open a Google popup either. The SDK module is replaced and each
// test says what the popup returned.
vi.mock('../../firebase/config', () => ({
  firebaseConfig: {},
  isFirebaseEnabled: vi.fn(() => true),
}))
vi.mock('../../firebase/firebase', () => ({
  signInWithGoogle: vi.fn(),
  getGoogleRedirectResult: vi.fn().mockResolvedValue(null),
}))

/** What POST /api/auth/firebase answers for a Google account it just created. */
const NEWCOMER_LOGIN: FirebaseLoginResponse = {
  csrf_token: 'csrf-1',
  jwt_token: 'jwt-1',
  is_new_user: true,
  user: {
    id: 'user-new',
    user_id: 'user-new',
    email: 'newcomer@example.com',
    display_name: 'New Comer',
    role: 'user',
    is_admin: false,
    user_status: 'active',
    tier: 'starter',
    tenant_id: 'user-new',
    created_at: '2026-10-08T12:00:00Z',
  },
}

/** Who AuthProvider holds, so a test can read the sign-in's result rather than a call count. */
function SignedInAs() {
  const { user } = useAuth()
  return <p data-testid="signed-in-as">{user ? user.email : 'nobody'}</p>
}

async function renderRegister() {
  await act(async () => {
    render(
      <AuthProvider>
        <Register onNavigateToLogin={() => {}} onRegistrationSuccess={() => {}} />
        <SignedInAs />
      </AuthProvider>,
    )
  })
}

describe('Register — Continue with Google', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    localStorage.clear()
    // clearAllMocks keeps implementations; one test turns Firebase off.
    const { isFirebaseEnabled } = await import('../../firebase/config')
    vi.mocked(isFirebaseEnabled).mockReturnValue(true)
  })

  it('signs a newcomer in with the ID token the Google popup returned', async () => {
    const user = userEvent.setup()
    const { signInWithGoogle } = await import('../../firebase/firebase')
    const { authApi } = await import('../../services/api')
    vi.mocked(signInWithGoogle).mockResolvedValue('google-id-token')
    vi.mocked(authApi.loginWithFirebase).mockResolvedValue(NEWCOMER_LOGIN)

    await renderRegister()
    await user.click(screen.getByRole('button', { name: 'Continue with Google' }))

    await waitFor(() => expect(screen.getByTestId('signed-in-as')).toHaveTextContent('newcomer@example.com'))
    expect(authApi.loginWithFirebase).toHaveBeenCalledWith({ idToken: 'google-id-token' })
    expect(authApi.register).not.toHaveBeenCalled()
  })

  it("puts a failed Google sign-in in the form's banner, in words the athlete reads", async () => {
    const user = userEvent.setup()
    const { signInWithGoogle } = await import('../../firebase/firebase')
    vi.mocked(signInWithGoogle).mockRejectedValue(
      Object.assign(new Error('Firebase: Error (auth/network-request-failed).'), {
        code: 'auth/network-request-failed',
      }),
    )

    await renderRegister()
    await user.click(screen.getByRole('button', { name: 'Continue with Google' }))

    expect(await screen.findByText('Network error. Check your connection.')).toBeInTheDocument()
    // The SDK's own English never reaches the athlete.
    expect(screen.queryByText(/auth\/network-request-failed/)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()
    expect(screen.getByTestId('signed-in-as')).toHaveTextContent('nobody')
  })

  it('clears the banner as a Google sign-in starts, and says nothing when the popup is closed', async () => {
    const user = userEvent.setup()
    const { signInWithGoogle } = await import('../../firebase/firebase')
    vi.mocked(signInWithGoogle).mockRejectedValue({ code: 'auth/popup-closed-by-user' })

    await renderRegister()
    // Put something in the banner first: a mismatched confirmation.
    await user.type(screen.getByLabelText(/email address/i), 'newcomer@example.com')
    await user.type(screen.getByLabelText(/^password$/i), 'long-enough-1')
    await user.type(screen.getByLabelText(/confirm password/i), 'long-enough-2')
    await user.click(screen.getByRole('button', { name: /create account/i }))
    expect(await screen.findByText('Passwords do not match')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Continue with Google' }))

    await waitFor(() => expect(screen.queryByText('Passwords do not match')).not.toBeInTheDocument())
    expect(screen.getByRole('button', { name: 'Continue with Google' })).toBeEnabled()
    expect(screen.queryByText(/google sign-in failed/i)).not.toBeInTheDocument()
  })

  it('keeps the spinner up while the redirect fallback leaves the page', async () => {
    const user = userEvent.setup()
    const { signInWithGoogle } = await import('../../firebase/firebase')
    const { authApi } = await import('../../services/api')
    // Popup refused (in-app browser): the SDK redirects and resolves null.
    vi.mocked(signInWithGoogle).mockResolvedValue(null)

    await renderRegister()
    await user.click(screen.getByRole('button', { name: 'Continue with Google' }))

    await waitFor(() => expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled())
    expect(authApi.loginWithFirebase).not.toHaveBeenCalled()
  })

  it('offers no Google button when Firebase is not configured', async () => {
    const { isFirebaseEnabled } = await import('../../firebase/config')
    vi.mocked(isFirebaseEnabled).mockReturnValue(false)

    await renderRegister()

    expect(screen.getByRole('button', { name: /create account/i })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Continue with Google' })).not.toBeInTheDocument()
    expect(screen.queryByText('or')).not.toBeInTheDocument()
  })
})
