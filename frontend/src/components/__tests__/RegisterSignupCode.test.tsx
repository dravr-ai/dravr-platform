// ABOUTME: Tests the optional signup code on the create-account form: hidden behind a link, sent trimmed, omitted when empty
// ABOUTME: Also covers a code the /signup?code= link prefilled, and the server's refusal shown inline (carnet#1128)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import Register from '../Register'

vi.mock('../../services/api', () => ({
  authApi: {
    getSignupConfig: vi.fn(),
    register: vi.fn(),
  },
}))

vi.mock('../../firebase/config', () => ({
  isFirebaseEnabled: () => false,
}))

// The form reads the session's sign-in from AuthProvider; these cases never
// get a sign-in code back, so it is never called.
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ completeSignInWithCode: vi.fn() }),
}))

const { authApi } = await import('../../services/api')

function renderRegister(initialSignupCode?: string) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const onRegistrationSuccess = vi.fn()
  const view = render(
    <QueryClientProvider client={queryClient}>
      <Register
        onNavigateToLogin={() => {}}
        onRegistrationSuccess={onRegistrationSuccess}
        initialSignupCode={initialSignupCode}
      />
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

async function submit(container: HTMLElement) {
  const button = container.querySelector<HTMLButtonElement>('button[type="submit"]')!
  await waitFor(() => expect(button).toBeEnabled())
  fireEvent.click(button)
}

describe('Register signup code', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: true, turnstile_site_key: null })
    vi.mocked(authApi.register).mockResolvedValue({
      message: 'Account created',
      user_id: 'u-1',
      user_status: 'active',
    } as Awaited<ReturnType<typeof authApi.register>>)
  })

  it('keeps the field behind a link and sends no code when none was entered', async () => {
    const { container } = renderRegister()

    expect(screen.getByRole('button', { name: 'Have a signup code?' })).toBeInTheDocument()
    expect(container.querySelector('#signupCode')).toBeNull()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(authApi.register).toHaveBeenCalledTimes(1))
    expect(vi.mocked(authApi.register).mock.calls[0][0]).not.toHaveProperty('signup_code')
  })

  it('sends the entered code trimmed as signup_code', async () => {
    const { container } = renderRegister()

    fireEvent.click(screen.getByRole('button', { name: 'Have a signup code?' }))
    fireEvent.change(screen.getByLabelText('Signup code'), { target: { value: '  BETA-ABCD-EFGH-JKLM ' } })
    fillForm(container)
    await submit(container)

    await waitFor(() =>
      expect(authApi.register).toHaveBeenCalledWith(
        expect.objectContaining({ email: 'new@example.com', signup_code: 'BETA-ABCD-EFGH-JKLM' }),
      ),
    )
  })

  it('opens the field filled with the code the link carried', async () => {
    const { container } = renderRegister('COACH-1234')

    expect(screen.getByLabelText('Signup code')).toHaveValue('COACH-1234')
    expect(screen.queryByRole('button', { name: 'Have a signup code?' })).not.toBeInTheDocument()

    fillForm(container)
    await submit(container)

    await waitFor(() =>
      expect(authApi.register).toHaveBeenCalledWith(expect.objectContaining({ signup_code: 'COACH-1234' })),
    )
  })

  it("shows the server's refusal of an invalid code and keeps the code to fix", async () => {
    vi.mocked(authApi.register).mockRejectedValue({
      response: { status: 400, data: { message: "This code isn't valid. Check it and try again." } },
    })
    const { container, onRegistrationSuccess } = renderRegister('BETA-TYPO')

    fillForm(container)
    await submit(container)

    expect(await screen.findByRole('alert')).toHaveTextContent("This code isn't valid. Check it and try again.")
    expect(screen.getByLabelText('Signup code')).toHaveValue('BETA-TYPO')
    expect(onRegistrationSuccess).not.toHaveBeenCalled()
  })
})
