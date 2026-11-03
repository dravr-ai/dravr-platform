// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the web analytics-consent switch writes through userApi and then updates the session's user
// ABOUTME: App boots and stops PostHog on that user, so a grant or revoke must reach it at once; a failed write reverts and changes nothing

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import PrivacySettingsTab from '../PrivacySettingsTab'

const { mockUpdateAnalyticsConsent, mockUpdateUser, mockUseAuth } = vi.hoisted(() => ({
  mockUpdateAnalyticsConsent: vi.fn(),
  mockUpdateUser: vi.fn(),
  mockUseAuth: vi.fn(),
}))

vi.mock('../../services/api', () => ({
  userApi: { updateAnalyticsConsent: mockUpdateAnalyticsConsent },
}))
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => mockUseAuth() }))
// The two neighbouring sections read their own endpoints; they are not under test.
vi.mock('../AccountDeletionSection', () => ({ default: () => null }))
vi.mock('../settings/AiConsentSettings', () => ({ default: () => null }))

function renderTab() {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <PrivacySettingsTab />
    </QueryClientProvider>
  )
}

function signedInWith(consent: boolean) {
  mockUseAuth.mockReturnValue({
    user: { id: 'user-1', email: 'athlete@example.com', analytics_consent: consent },
    updateUser: mockUpdateUser,
  })
}

describe('PrivacySettingsTab — analytics consent', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('seeds the switch from the session user', () => {
    signedInWith(true)
    renderTab()

    expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true')
  })

  it('writes a grant and puts it on the session user, so analytics start now', async () => {
    signedInWith(false)
    mockUpdateAnalyticsConsent.mockResolvedValue({ message: 'Analytics consent updated', enabled: true })
    renderTab()

    fireEvent.click(screen.getByRole('switch'))

    await waitFor(() => expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: true }))
    expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(true)
    expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true')
  })

  it('writes a revoke and puts it on the session user, so analytics stop now', async () => {
    signedInWith(true)
    mockUpdateAnalyticsConsent.mockResolvedValue({ message: 'Analytics consent updated', enabled: false })
    renderTab()

    fireEvent.click(screen.getByRole('switch'))

    await waitFor(() => expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: false }))
    expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(false)
  })

  it('reverts the switch and leaves the session user alone when the write fails', async () => {
    signedInWith(false)
    mockUpdateAnalyticsConsent.mockRejectedValue(new Error('network down'))
    renderTab()

    fireEvent.click(screen.getByRole('switch'))

    await waitFor(() => expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'false'))
    expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(true)
    expect(mockUpdateUser).not.toHaveBeenCalled()
  })
})
