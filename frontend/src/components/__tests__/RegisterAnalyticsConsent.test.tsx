// ABOUTME: Tests the usage-analytics opt-in on the create-account form: its own box, unticked, sent either way (carnet#1366)
// ABOUTME: Also pins the link to the privacy policy's usage-analytics section beside it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { ANALYTICS_POLICY_URL } from '@pierre/shared-constants'
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

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ completeSignInWithCode: vi.fn() }),
}))

const { authApi } = await import('../../services/api')

function renderRegister() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={queryClient}>
      <Register onNavigateToLogin={() => {}} onRegistrationSuccess={vi.fn()} />
    </QueryClientProvider>,
  )
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

describe('Register analytics consent', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: true, turnstile_site_key: null })
    vi.mocked(authApi.register).mockResolvedValue({
      message: 'Account created',
      user_id: 'u-1',
      user_status: 'active',
    } as Awaited<ReturnType<typeof authApi.register>>)
  })

  it('asks with its own box, unticked, and links the policy section', () => {
    renderRegister()

    const box = screen.getByRole('checkbox', { name: /Help improve Dravr with usage analytics/ })
    expect(box).not.toBeChecked()
    expect(screen.getByRole('link', { name: 'How we use usage analytics' })).toHaveAttribute(
      'href',
      ANALYTICS_POLICY_URL,
    )
  })

  it('sends a no when the box is left unticked', async () => {
    const { container } = renderRegister()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(authApi.register).toHaveBeenCalledTimes(1))
    expect(vi.mocked(authApi.register).mock.calls[0][0]).toMatchObject({ analytics_consent: false })
  })

  it('sends a yes when the box is ticked', async () => {
    const { container } = renderRegister()

    fillForm(container)
    fireEvent.click(screen.getByRole('checkbox', { name: /Help improve Dravr with usage analytics/ }))
    await submit(container)

    await waitFor(() => expect(authApi.register).toHaveBeenCalledTimes(1))
    expect(vi.mocked(authApi.register).mock.calls[0][0]).toMatchObject({ analytics_consent: true })
  })
})
