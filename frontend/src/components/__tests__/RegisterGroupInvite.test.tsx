// ABOUTME: Tests the create-account form sending the group invite this tab carries, for the server to hold (carnet#1648)
// ABOUTME: The carry outlives the sign-up, whose hold is best-effort, until a session's hold answers; it never fills the signup code field
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor, fireEvent } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import Register from '../Register'
import { captureGroupInviteLink, readCarriedGroupInvite } from '../../utils/groupInvite'

vi.mock('../../services/api', () => ({
  authApi: {
    getSignupConfig: vi.fn(),
    register: vi.fn(),
  },
}))

vi.mock('../../firebase/config', () => ({
  isFirebaseEnabled: () => false,
}))

// These cases never get a sign-in code back, so the session's sign-in is never called.
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ completeSignInWithCode: vi.fn() }),
}))

const { authApi } = await import('../../services/api')

function renderRegister() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const onRegistrationSuccess = vi.fn()
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

async function submit(container: HTMLElement) {
  const button = container.querySelector<HTMLButtonElement>('button[type="submit"]')!
  await waitFor(() => expect(button).toBeEnabled())
  fireEvent.click(button)
}

/** Open the page on an invite link, as an athlete following a coach's link does. */
function openInviteLink(code: string) {
  window.history.replaceState(null, '', `/groups/join/${code}`)
  captureGroupInviteLink()
}

describe('Register with a group invite link', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    sessionStorage.clear()
    window.history.replaceState(null, '', '/')
    vi.mocked(authApi.getSignupConfig).mockResolvedValue({ open: true, turnstile_site_key: null })
    vi.mocked(authApi.register).mockResolvedValue({
      message: 'Check your inbox',
      user_id: 'u-1',
      user_status: 'pending',
    } as Awaited<ReturnType<typeof authApi.register>>)
  })

  it('sends the carried code as group_invite_code and keeps carrying it until a session holds it', async () => {
    openInviteLink('MRT2026X')
    expect(window.location.pathname).toBe('/')
    const { container, onRegistrationSuccess } = renderRegister()

    // The invite is not a signup code: the field stays behind its link.
    expect(container.querySelector('#signupCode')).toBeNull()
    fillForm(container)
    await submit(container)

    await waitFor(() => expect(onRegistrationSuccess).toHaveBeenCalledTimes(1))
    const body = vi.mocked(authApi.register).mock.calls[0][0]
    expect(body.group_invite_code).toBe('MRT2026X')
    expect(body).not.toHaveProperty('signup_code')
    // The registration's hold is best-effort: the session's hold drops it.
    expect(readCarriedGroupInvite()).toBe('MRT2026X')
  })

  it('keeps the carry when the sign-up fails, for the next attempt', async () => {
    vi.mocked(authApi.register).mockRejectedValue({
      response: { status: 429, data: { message: 'Too many sign-ups from this network.' } },
    })
    openInviteLink('MRT2026X')
    const { container, onRegistrationSuccess } = renderRegister()

    fillForm(container)
    await submit(container)

    expect(await screen.findByRole('alert')).toHaveTextContent('Too many sign-ups from this network.')
    expect(onRegistrationSuccess).not.toHaveBeenCalled()
    expect(readCarriedGroupInvite()).toBe('MRT2026X')
  })

  it('sends no group_invite_code when the tab carries none', async () => {
    const { container, onRegistrationSuccess } = renderRegister()

    fillForm(container)
    await submit(container)

    await waitFor(() => expect(onRegistrationSuccess).toHaveBeenCalledTimes(1))
    expect(vi.mocked(authApi.register).mock.calls[0][0]).not.toHaveProperty('group_invite_code')
  })
})
