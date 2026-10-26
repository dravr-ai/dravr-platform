// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the create-account form's straight sign-in (carnet#1147): an active account's code is redeemed with the kept verifier
// ABOUTME: No code, a refused exchange, or an account that waits on email or review all fall back to the sign-in screen, never an error

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor, act, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { RegisterResponse } from '@pierre/shared-types'
import Register from '../Register'
import { AuthProvider } from '../../contexts/AuthContext'
import { useAuth } from '../../hooks/useAuth'
import { PENDING_SIGN_IN_KEY } from '../../utils/signIn'

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

// The HTTP layer only: AuthProvider's real completeSignInWithCode runs and
// establishes the session. The browser round trip is in e2e/registration.spec.ts.
vi.mock('../../services/api', () => ({
  authApi: {
    getSignupConfig: vi.fn(),
    register: vi.fn(),
    prepareRegistrationSignIn: vi.fn(),
    completeSignIn: vi.fn(),
    loginWithFirebase: vi.fn(),
    beginSignIn: vi.fn(),
    logout: vi.fn().mockResolvedValue(undefined),
    getSession: vi.fn(),
  },
  adminApi: { endImpersonation: vi.fn() },
  userApi: { setTimezone: vi.fn().mockResolvedValue(undefined) },
  pierreApi: { adapter: { authStorage: mockAuthStorage } },
}))

vi.mock('../../firebase/config', () => ({
  isFirebaseEnabled: () => false,
}))

const { authApi } = await import('../../services/api')

const REDIRECT_URI = `${window.location.origin}/auth/callback`

/** What prepareRegistrationSignIn hands the form for submit number `n`. */
function preparedSignIn(n: number) {
  return {
    request: {
      client_id: 'dravr-web',
      redirect_uri: REDIRECT_URI,
      code_challenge: `challenge-${n}`,
      code_challenge_method: 'S256' as const,
      state: `state-${n}`,
    },
    codeVerifier: `verifier-${n}`,
    redirectUri: REDIRECT_URI,
  }
}

const NEW_ATHLETE = {
  id: 'user-new',
  user_id: 'user-new',
  email: 'new@example.com',
  display_name: 'New Athlete',
  role: 'user',
  is_admin: false,
  user_status: 'active',
  tier: 'starter',
  tenant_id: 'user-new',
  created_at: '2026-10-26T12:00:00Z',
}

function registered(activation: RegisterResponse['activation'], signInCode?: string): RegisterResponse {
  return {
    user_id: 'user-new',
    user_status: activation === 'active' ? 'active' : 'pending',
    activation,
    message: 'Account created',
    ...(signInCode ? { sign_in_code: signInCode } : {}),
  } as RegisterResponse
}

function SignedInAs() {
  const { user } = useAuth()
  return <p data-testid="signed-in-as">{user ? user.email : 'nobody'}</p>
}

async function renderRegister() {
  const onRegistrationSuccess = vi.fn()
  let view!: ReturnType<typeof render>
  await act(async () => {
    view = render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <AuthProvider>
          <Register onNavigateToLogin={() => {}} onRegistrationSuccess={onRegistrationSuccess} />
          <SignedInAs />
        </AuthProvider>
      </QueryClientProvider>,
    )
  })
  return { container: view.container, onRegistrationSuccess }
}

function fillForm(container: HTMLElement) {
  const field = (id: string) => container.querySelector<HTMLInputElement>(`#${id}`)!
  fireEvent.change(field('email'), { target: { value: 'new@example.com' } })
  fireEvent.change(field('password'), { target: { value: 'password123' } })
  fireEvent.change(field('confirmPassword'), { target: { value: 'password123' } })
}

async function submit(container: HTMLElement) {
  const button = container.querySelector<HTMLButtonElement>('button[type="submit"]')!
  await waitFor(() => expect(button).toBeEnabled())
  fireEvent.click(button)
}

describe('Register — signed straight in', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
    sessionStorage.clear()
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: true, turnstile_site_key: null })
    let prepared = 0
    vi.mocked(authApi.prepareRegistrationSignIn).mockImplementation(async () => preparedSignIn(++prepared))
  })

  it('redeems an active account’s code with the kept verifier and signs in without the banner', async () => {
    vi.mocked(authApi.register).mockResolvedValue(registered('active', 'the-code'))
    vi.mocked(authApi.completeSignIn).mockResolvedValue({
      access_token: 'jwt-1',
      csrf_token: 'csrf-1',
      user: NEW_ATHLETE,
    } as Awaited<ReturnType<typeof authApi.completeSignIn>>)
    const { container, onRegistrationSuccess } = await renderRegister()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(screen.getByTestId('signed-in-as')).toHaveTextContent('new@example.com'))
    expect(authApi.prepareRegistrationSignIn).toHaveBeenCalledWith(REDIRECT_URI, expect.anything())
    expect(authApi.register).toHaveBeenCalledWith(
      expect.objectContaining({ email: 'new@example.com', sign_in: preparedSignIn(1).request }),
    )
    expect(authApi.completeSignIn).toHaveBeenCalledWith({
      code: 'the-code',
      codeVerifier: 'verifier-1',
      redirectUri: REDIRECT_URI,
    })
    expect(mockAuthStorage.setCsrfToken).toHaveBeenCalledWith('csrf-1')
    expect(onRegistrationSuccess).not.toHaveBeenCalled()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    // The code never went through /auth/callback, so no pending sign-in is left for it.
    expect(sessionStorage.getItem(PENDING_SIGN_IN_KEY)).toBeNull()
  })

  it('falls back to the sign-in screen when the exchange is refused, with no error', async () => {
    vi.mocked(authApi.register).mockResolvedValue(registered('active', 'the-code'))
    vi.mocked(authApi.completeSignIn).mockRejectedValue({ response: { status: 400, data: { error: 'invalid_grant' } } })
    const { container, onRegistrationSuccess } = await renderRegister()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(onRegistrationSuccess).toHaveBeenCalledWith(expect.any(String), 'new@example.com'))
    expect(authApi.completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ codeVerifier: 'verifier-1' }))
    expect(screen.getByTestId('signed-in-as')).toHaveTextContent('nobody')
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('falls back to the sign-in screen when an active account comes back without a code', async () => {
    vi.mocked(authApi.register).mockResolvedValue(registered('active'))
    const { container, onRegistrationSuccess } = await renderRegister()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(onRegistrationSuccess).toHaveBeenCalledWith(expect.any(String), 'new@example.com'))
    expect(authApi.completeSignIn).not.toHaveBeenCalled()
  })

  it.each(['confirm_email', 'review'] as const)('keeps today’s screen for a %s account', async (activation) => {
    // Even a stray code is not redeemed for an account that is not active.
    vi.mocked(authApi.register).mockResolvedValue(registered(activation, 'stray-code'))
    const { container, onRegistrationSuccess } = await renderRegister()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(onRegistrationSuccess).toHaveBeenCalledWith(expect.any(String), 'new@example.com'))
    expect(authApi.completeSignIn).not.toHaveBeenCalled()
    expect(screen.getByTestId('signed-in-as')).toHaveTextContent('nobody')
  })

  it('prepares a fresh sign-in for every submit', async () => {
    vi.mocked(authApi.register)
      .mockRejectedValueOnce({ response: { status: 503, data: { message: 'Try again' } } })
      .mockResolvedValueOnce(registered('review'))
    const { container, onRegistrationSuccess } = await renderRegister()

    fillForm(container)
    await submit(container)
    expect(await screen.findByRole('alert')).toHaveTextContent('Try again')
    await submit(container)

    await waitFor(() => expect(onRegistrationSuccess).toHaveBeenCalled())
    expect(authApi.prepareRegistrationSignIn).toHaveBeenCalledTimes(2)
    expect(vi.mocked(authApi.register).mock.calls[0][0].sign_in?.state).toBe('state-1')
    expect(vi.mocked(authApi.register).mock.calls[1][0].sign_in?.state).toBe('state-2')
  })

  it('still registers when the browser cannot prepare a sign-in', async () => {
    vi.mocked(authApi.prepareRegistrationSignIn).mockRejectedValue(new Error('crypto.subtle is undefined'))
    vi.mocked(authApi.register).mockResolvedValue(registered('active'))
    const { container, onRegistrationSuccess } = await renderRegister()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(onRegistrationSuccess).toHaveBeenCalled())
    expect(vi.mocked(authApi.register).mock.calls[0][0]).not.toHaveProperty('sign_in')
  })
})
