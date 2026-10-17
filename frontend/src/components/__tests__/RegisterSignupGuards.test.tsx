// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the create-account form against the sign-up guards: a paused notice, and the Turnstile token
// ABOUTME: window.turnstile is a stub, so no Cloudflare script is ever fetched (carnet#1063)

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { render, screen, waitFor, act, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import Register from '../Register'
import type { TurnstileApi, TurnstileRenderOptions } from '../TurnstileWidget'

vi.mock('../../services/api', () => ({
  authApi: {
    getSignupConfig: vi.fn(),
    register: vi.fn(),
  },
}))

vi.mock('../../firebase/config', () => ({
  isFirebaseEnabled: () => false,
}))

const { authApi } = await import('../../services/api')

/** Cloudflare's always-pass test site key. */
const TEST_SITE_KEY = '1x00000000000000000000AA'

function renderRegister(onRegistrationSuccess = vi.fn()) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(
    <QueryClientProvider client={queryClient}>
      <Register onNavigateToLogin={() => {}} onRegistrationSuccess={onRegistrationSuccess} />
    </QueryClientProvider>,
  )
  return { ...view, onRegistrationSuccess }
}

function fillForm(container: HTMLElement) {
  const field = (id: string) => container.querySelector<HTMLInputElement>(`#${id}`)!
  fireEvent.change(field('email'), { target: { value: 'new@example.com' } })
  fireEvent.change(field('password'), { target: { value: 'password123' } })
  fireEvent.change(field('confirmPassword'), { target: { value: 'password123' } })
}

function submitButton(container: HTMLElement): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>('button[type="submit"]')!
}

describe('Register sign-up guards', () => {
  let rendered: TurnstileRenderOptions | null
  let turnstile: { [K in keyof TurnstileApi]: ReturnType<typeof vi.fn> }

  beforeEach(() => {
    vi.clearAllMocks()
    rendered = null
    turnstile = {
      render: vi.fn((_el: HTMLElement, options: TurnstileRenderOptions) => {
        rendered = options
        return 'widget-1'
      }),
      reset: vi.fn(),
      remove: vi.fn(),
    }
    window.turnstile = turnstile as unknown as TurnstileApi
  })

  afterEach(() => {
    delete window.turnstile
  })

  it('shows the paused notice instead of the form while sign-up is closed', async () => {
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: false, turnstile_site_key: null })

    const { container } = renderRegister()

    expect(await screen.findByTestId('signup-paused')).toBeInTheDocument()
    expect(container.querySelector('form')).toBeNull()
    expect(container.querySelector('#email')).toBeNull()
  })

  it('keeps the plain form when the sign-up config cannot be read', async () => {
    vi.mocked(authApi.getSignupConfig).mockRejectedValue(new Error('404'))

    const { container } = renderRegister()

    // The read is retried once (a 1 s backoff) before the form gives up on it.
    await waitFor(() => expect(authApi.getSignupConfig).toHaveBeenCalledTimes(2), { timeout: 3000 })
    await waitFor(() => expect(submitButton(container)).toBeEnabled())
    expect(screen.queryByTestId('signup-paused')).toBeNull()
    expect(screen.queryByTestId('turnstile-widget')).toBeNull()
  })

  it('holds submit until Turnstile passes, then sends its token', async () => {
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: true, turnstile_site_key: TEST_SITE_KEY })
    vi.mocked(authApi.register).mockResolvedValue({
      user_id: 'u1',
      email: 'new@example.com',
      user_status: 'pending',
      message: 'Confirm your email to activate your account.',
    })

    const { container, onRegistrationSuccess } = renderRegister()

    await waitFor(() => expect(turnstile.render).toHaveBeenCalled())
    expect(rendered?.sitekey).toBe(TEST_SITE_KEY)
    fillForm(container)
    expect(submitButton(container)).toBeDisabled()

    act(() => rendered?.callback('tok-1'))
    await waitFor(() => expect(submitButton(container)).toBeEnabled())

    fireEvent.click(submitButton(container))

    await waitFor(() =>
      expect(authApi.register).toHaveBeenCalledWith({
        email: 'new@example.com',
        password: 'password123',
        display_name: undefined,
        turnstile_token: 'tok-1',
      }),
    )
    // The server's sentence is what the next screen shows, not a hardcoded one.
    await waitFor(() =>
      expect(onRegistrationSuccess).toHaveBeenCalledWith(
        'Confirm your email to activate your account.',
        'new@example.com',
      ),
    )
  })

  it('shows the refusal and asks for a fresh check after a failed human check', async () => {
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: true, turnstile_site_key: TEST_SITE_KEY })
    vi.mocked(authApi.register).mockRejectedValue({
      response: { status: 403, data: { code: 'PermissionDenied', message: 'The human check failed. Please try again.' } },
    })

    const { container } = renderRegister()

    await waitFor(() => expect(turnstile.render).toHaveBeenCalled())
    fillForm(container)
    act(() => rendered?.callback('tok-spent'))
    await waitFor(() => expect(submitButton(container)).toBeEnabled())

    fireEvent.click(submitButton(container))

    expect(await screen.findByRole('alert')).toHaveTextContent('The human check failed. Please try again.')
    expect(turnstile.reset).toHaveBeenCalledWith('widget-1')
    // The spent token is gone: submit waits for the next one.
    expect(submitButton(container)).toBeDisabled()
  })

  it('shows the server sentence when sign-up was paused after the form loaded', async () => {
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: true, turnstile_site_key: null })
    vi.mocked(authApi.register).mockRejectedValue({
      response: {
        status: 503,
        data: { code: 'ResourceUnavailable', message: 'Sign-up is paused right now. Please try again later.' },
      },
    })

    const { container } = renderRegister()

    await waitFor(() => expect(submitButton(container)).toBeEnabled())
    fillForm(container)
    fireEvent.click(submitButton(container))

    expect(await screen.findByRole('alert')).toHaveTextContent('Sign-up is paused right now. Please try again later.')
  })
})
