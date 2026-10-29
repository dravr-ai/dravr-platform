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

  it('should render error state for failed connection', () => {
    render(<OAuthCallback provider="strava" onClose={vi.fn()} />)

    expect(screen.getByText('Connection Failed')).toBeInTheDocument()
  })

  it('says the whole failure sentence in French, provider name inside it', async () => {
    await i18n.changeLanguage('fr')
    try {
      render(<OAuthCallback provider="strava" onClose={vi.fn()} />)

      expect(screen.getByText('La connexion a échoué')).toBeInTheDocument()
      expect(screen.getByText('Impossible de connecter ton compte Strava. Réessaie.')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Continuer' })).toBeInTheDocument()
    } finally {
      await i18n.changeLanguage('en')
    }
  })

  it('should display custom error message when provided', () => {
    render(<OAuthCallback provider="garmin" error="Invalid token" onClose={vi.fn()} />)

    expect(screen.getByText('Invalid token')).toBeInTheDocument()
  })

  it('should display default error message when no error provided', () => {
    render(<OAuthCallback provider="strava" onClose={vi.fn()} />)

    expect(screen.getByText(/Failed to connect your Strava account/)).toBeInTheDocument()
  })

  it('should capitalize provider name', () => {
    render(<OAuthCallback provider="whoop" onClose={vi.fn()} />)

    expect(screen.getByText(/Failed to connect your Whoop account/)).toBeInTheDocument()
  })

  it('goes back to where the connect started through onClose, in this same tab', async () => {
    const onClose = vi.fn()
    const open = vi.spyOn(window, 'open').mockReturnValue(null)
    const user = userEvent.setup()
    try {
      render(<OAuthCallback provider="whoop" error="access_denied" onClose={onClose} />)

      await user.click(screen.getByRole('button', { name: 'Continue' }))
      expect(onClose).toHaveBeenCalledTimes(1)
      // No second copy of the app: the button neither opens a tab nor names a dashboard.
      expect(open).not.toHaveBeenCalled()
      expect(screen.queryByText(/Dashboard/)).not.toBeInTheDocument()
      expect(screen.queryByText(/close this tab/)).not.toBeInTheDocument()
    } finally {
      open.mockRestore()
    }
  })

  it('leaves the result to settleOAuthReturn and writes nothing itself', () => {
    render(<OAuthCallback provider="garmin" error="Token expired" onClose={vi.fn()} />)

    expect(localStorage.getItem('pierre_oauth_result')).toBeNull()
  })

  it('words a tombstoned account with its deletion date and offers to start fresh', async () => {
    render(<OAuthCallback provider="strava" error="import_tombstoned" tombstone={TOMBSTONE} onClose={vi.fn()} />)

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
      render(<OAuthCallback provider="strava" error="import_tombstoned" tombstone={TOMBSTONE} onClose={vi.fn()} />)

      await user.click(screen.getByRole('button', { name: 'Start fresh and connect' }))

      expect(userApi.clearImportTombstone).toHaveBeenCalledWith('tomb-1')
      await waitFor(() => expect(assign).toHaveBeenCalledWith('/api/oauth/strava/start'))
    } finally {
      Object.defineProperty(window, 'location', { configurable: true, value: original })
    }
  })

  it('should show Dravr branding', () => {
    render(<OAuthCallback provider="strava" onClose={vi.fn()} />)

    expect(screen.getByText('Dravr')).toBeInTheDocument()
  })
})
