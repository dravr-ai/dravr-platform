// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import type { ReactElement } from 'react'
import { render as renderRaw, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { i18n } from '@pierre/i18n'

vi.mock('../../services/api', () => ({
  oauthApi: { authorizeUrl: vi.fn((provider: string) => `/api/oauth/${provider}/start`) },
  providersApi: {
    getProvidersStatus: vi.fn().mockResolvedValue({
      providers: [{ provider: 'strava', display_name: 'Strava', connected: false }],
    }),
  },
  userApi: { clearImportTombstone: vi.fn() },
}))

import OAuthCallback from '../OAuthCallback'
import { userApi } from '../../services/api'

/** The page renders inside the app's query client, as App mounts it. */
function render(ui: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return renderRaw(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

const TOMBSTONE = { tombstoneId: 'tomb-1', deletedAt: '2026-10-20T08:00:00+00:00' }

describe('OAuthCallback', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('should render success state for successful connection', () => {
    render(<OAuthCallback provider="strava" success={true} />)

    expect(screen.getByText('Strava connected successfully!')).toBeInTheDocument()
    expect(
      screen.getByText('Your Strava account has been successfully connected to Dravr.'),
    ).toBeInTheDocument()
  })

  it('says the whole success sentence in French, provider name inside it', async () => {
    // The body used to be a translated "Your" in front of an English sentence
    // — "Ton Strava account has been successfully connected to Dravr."
    await i18n.changeLanguage('fr')
    try {
      render(<OAuthCallback provider="strava" success={true} />)

      expect(screen.getByText('Strava connecté !')).toBeInTheDocument()
      expect(screen.getByText('Ton compte Strava est bien connecté à Dravr.')).toBeInTheDocument()
    } finally {
      await i18n.changeLanguage('en')
    }
  })

  it('should render error state for failed connection', () => {
    render(<OAuthCallback provider="strava" success={false} />)

    expect(screen.getByText('Connection Failed')).toBeInTheDocument()
  })

  it('should display custom error message when provided', () => {
    render(<OAuthCallback provider="garmin" success={false} error="Invalid token" />)

    expect(screen.getByText('Invalid token')).toBeInTheDocument()
  })

  it('should display default error message when no error provided', () => {
    render(<OAuthCallback provider="strava" success={false} />)

    expect(screen.getByText(/Failed to connect your Strava account/)).toBeInTheDocument()
  })

  it('should capitalize provider name', () => {
    render(<OAuthCallback provider="strava" success={true} />)

    expect(screen.getByText('Strava connected successfully!')).toBeInTheDocument()
  })

  it('should show Continue button when onClose is provided', async () => {
    const onClose = vi.fn()
    const user = userEvent.setup()

    render(<OAuthCallback provider="strava" success={true} onClose={onClose} />)

    const button = screen.getByText('Continue to Dashboard')
    expect(button).toBeInTheDocument()

    await user.click(button)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('should show close tab message when onClose is not provided', () => {
    render(<OAuthCallback provider="strava" success={true} />)

    expect(screen.getByText(/You can close this tab/)).toBeInTheDocument()
    expect(screen.queryByText('Continue to Dashboard')).not.toBeInTheDocument()
  })

  it('should store OAuth result in localStorage on success', () => {
    render(<OAuthCallback provider="strava" success={true} />)

    const stored = JSON.parse(localStorage.getItem('pierre_oauth_result') || '{}')
    expect(stored.type).toBe('oauth_completed')
    expect(stored.provider).toBe('strava')
    expect(stored.success).toBe(true)
    expect(stored.hasError).toBe(false)
  })

  it('should store OAuth result in localStorage on failure', () => {
    render(<OAuthCallback provider="garmin" success={false} error="Token expired" />)

    const stored = JSON.parse(localStorage.getItem('pierre_oauth_result') || '{}')
    expect(stored.type).toBe('oauth_completed')
    expect(stored.provider).toBe('garmin')
    expect(stored.success).toBe(false)
    expect(stored.hasError).toBe(true)
  })

  it('words a tombstoned account with its deletion date and offers to start fresh', async () => {
    render(<OAuthCallback provider="strava" success={false} error="import_tombstoned" tombstone={TOMBSTONE} />)

    expect(
      await screen.findByText(
        /This Strava account belonged to a Dravr account deleted on Oct 20, 2026\./,
      ),
    ).toBeInTheDocument()
    // The raw code is never the sentence.
    expect(screen.queryByText('import_tombstoned')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Start fresh and connect' })).toBeInTheDocument()
  })

  it('clears the tombstone the return named, then runs the connect again in this tab', async () => {
    vi.mocked(userApi.clearImportTombstone).mockResolvedValue({ cleared: [], user_status: 'active' })
    const assign = vi.fn()
    const original = window.location
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: { ...original, set href(url: string) { assign(url) } },
    })
    try {
      const user = userEvent.setup()
      render(<OAuthCallback provider="strava" success={false} error="import_tombstoned" tombstone={TOMBSTONE} />)

      await user.click(screen.getByRole('button', { name: 'Start fresh and connect' }))

      expect(userApi.clearImportTombstone).toHaveBeenCalledWith('tomb-1')
      await waitFor(() => expect(assign).toHaveBeenCalledWith('/api/oauth/strava/start'))
    } finally {
      Object.defineProperty(window, 'location', { configurable: true, value: original })
    }
  })

  it('marks a tombstoned result so the opener does not fall back to another way in', () => {
    render(<OAuthCallback provider="strava" success={false} error="import_tombstoned" tombstone={TOMBSTONE} />)

    const stored = JSON.parse(localStorage.getItem('pierre_oauth_result') || '{}')
    expect(stored.success).toBe(false)
    expect(stored.tombstoned).toBe(true)
  })

  it('should show Dravr branding', () => {
    render(<OAuthCallback provider="strava" success={true} />)

    expect(screen.getByText('Dravr')).toBeInTheDocument()
  })
})
