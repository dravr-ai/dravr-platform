// ABOUTME: Tests the Sign-up block of AdminSettings: the kill switch, the daily ceiling, activate-on-verified-email
// ABOUTME: Asserts each control sends only the field it changes, and the read-only figures render (carnet#1063)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import AdminSettings from '../AdminSettings'
import type { SignupSetting } from '../../services/api/admin'

vi.mock('../../services/api', () => ({
  adminApi: {
    getAutoApprovalSetting: vi.fn(() =>
      Promise.resolve({ enabled: false, description: 'Auto-approval', overridden_by_env: false }),
    ),
    updateAutoApprovalSetting: vi.fn(),
    getSignupSetting: vi.fn(),
    updateSignupSetting: vi.fn(),
    updateConfig: vi.fn(),
  },
}))

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { tenant_id: null } }),
}))

vi.mock('../../hooks/useGroups', () => ({
  useGroupPermissions: () => ({ policy: 'admins_only', isLoading: false }),
}))

vi.mock('../FeatureFlagsPanel', () => ({
  default: () => <div data-testid="feature-flags-panel" />,
}))

const { adminApi } = await import('../../services/api')

const OPEN: SignupSetting = {
  open: true,
  daily_limit: 50,
  activate_on_verified_email: false,
  created_last_24h: 12,
  turnstile_configured: true,
  description: 'Who may create an account, and how many per day.',
}

function renderSettings() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={queryClient}>
      <AdminSettings />
    </QueryClientProvider>,
  )
}

describe('AdminSettings sign-up guards', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(adminApi.getSignupSetting).mockResolvedValue(OPEN)
  })

  it('renders the figures the server reports', async () => {
    renderSettings()

    expect(await screen.findByTestId('signup-created-24h')).toHaveTextContent('12 / 50')
    expect(screen.getByTestId('signup-turnstile-configured')).toHaveTextContent('Yes')
    expect(screen.getByText(OPEN.description)).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: 'Sign-up open' })).toHaveAttribute('aria-checked', 'true')
  })

  it('pauses sign-up with the kill switch, sending only `open`', async () => {
    vi.mocked(adminApi.updateSignupSetting).mockResolvedValue({ ...OPEN, open: false })
    renderSettings()

    fireEvent.click(await screen.findByRole('switch', { name: 'Sign-up open' }))

    await waitFor(() => expect(adminApi.updateSignupSetting).toHaveBeenCalledWith({ open: false }))
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: 'Sign-up open' })).toHaveAttribute('aria-checked', 'false'),
    )
  })

  it('flips activate-on-verified-email, sending only that field', async () => {
    vi.mocked(adminApi.updateSignupSetting).mockResolvedValue({ ...OPEN, activate_on_verified_email: true })
    renderSettings()

    fireEvent.click(await screen.findByRole('switch', { name: 'Activate on verified email' }))

    await waitFor(() =>
      expect(adminApi.updateSignupSetting).toHaveBeenCalledWith({ activate_on_verified_email: true }),
    )
  })

  it('saves a new daily ceiling, 0 meaning none, and refuses a value that is not a whole number', async () => {
    vi.mocked(adminApi.updateSignupSetting).mockResolvedValue({ ...OPEN, daily_limit: 0 })
    renderSettings()

    const input = await screen.findByLabelText('Daily sign-up limit')
    const save = screen.getByRole('button', { name: 'Save limit' })
    expect(save).toBeDisabled()

    fireEvent.change(input, { target: { value: '-3' } })
    expect(save).toBeDisabled()
    expect(screen.getByText('Enter a whole number, 0 or more.')).toBeInTheDocument()

    fireEvent.change(input, { target: { value: '0' } })
    expect(save).toBeEnabled()
    fireEvent.click(save)

    await waitFor(() => expect(adminApi.updateSignupSetting).toHaveBeenCalledWith({ daily_limit: 0 }))
    // With no ceiling the count stands alone.
    await waitFor(() => expect(screen.getByTestId('signup-created-24h')).toHaveTextContent(/^12$/))
  })
})
