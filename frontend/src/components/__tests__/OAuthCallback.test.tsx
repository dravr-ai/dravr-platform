// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import OAuthCallback from '../OAuthCallback'

describe('OAuthCallback', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('should render success state for successful connection', () => {
    render(<OAuthCallback provider="strava" success={true} />)

    expect(screen.getByText('Strava Connected')).toBeInTheDocument()
    expect(screen.getByText(/successfully connected/)).toBeInTheDocument()
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

    expect(screen.getByText('Strava Connected')).toBeInTheDocument()
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

  it('should show Dravr branding', () => {
    render(<OAuthCallback provider="strava" success={true} />)

    expect(screen.getByText('Dravr')).toBeInTheDocument()
  })
})
