// ABOUTME: Tests the admin console's new-user view: funnel counts, one row per account, the window picker
// ABOUTME: Also pins the pure helpers: time since sign-up and the percentage of a whole

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import NewUsersView from '../NewUsersView'
import { percentOf, sinceSignup } from '../../utils/newUserCohort'
import type { NewUserCohort } from '../../services/api/admin'

// The view is tested against a canned report so each test controls the
// counts, rows and failures it renders without a server; the endpoint itself
// runs against a real database in new_user_cohort_test.rs.
vi.mock('../../services/api', () => ({
  adminApi: {
    getNewUsers: vi.fn(),
  },
}))

const { adminApi } = await import('../../services/api')

const COHORT: NewUserCohort = {
  window_days: 30,
  generated_at: '2026-11-03T12:00:00Z',
  truncated: false,
  funnel: {
    signed_up: 4,
    approved_or_active: 3,
    email_verified: 2,
    provider_connected: 2,
    first_sync: 1,
    first_turn: 1,
    day_7_eligible: 2,
    active_after_day_7: 1,
  },
  users: [
    {
      user_id: 'u-1',
      email: 'ana@example.com',
      display_name: 'Ana',
      signed_up_at: '2026-11-01T09:00:00Z',
      signup_source: 'google',
      status: 'active',
      approved_at: '2026-11-01T09:00:00Z',
      email_verified: true,
      provider_connected_at: '2026-11-01T09:30:00Z',
      first_sync_at: '2026-11-01T12:00:00Z',
      first_turn_at: '2026-11-03T09:00:00Z',
      turns_first_week: 5,
      last_active_at: '2026-11-03T10:00:00Z',
    },
    {
      user_id: 'u-2',
      email: 'ben@example.com',
      display_name: null,
      signed_up_at: '2026-10-20T09:00:00Z',
      signup_source: 'email',
      status: 'pending',
      approved_at: null,
      email_verified: false,
      provider_connected_at: null,
      first_sync_at: null,
      first_turn_at: null,
      turns_first_week: 0,
      last_active_at: null,
    },
  ],
}

function renderView() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <NewUsersView />
    </QueryClientProvider>,
  )
}

/** A stand-in for `t`: the key, then its interpolation values. */
describe('NewUsersView', () => {
  beforeEach(() => {
    vi.mocked(adminApi.getNewUsers).mockReset()
    vi.mocked(adminApi.getNewUsers).mockResolvedValue(COHORT)
  })

  it('asks for the last 30 days by default and shows the funnel counts', async () => {
    renderView()

    expect(await screen.findByTestId('new-users-funnel')).toBeInTheDocument()
    expect(adminApi.getNewUsers).toHaveBeenCalledWith(30)
    expect(screen.getByTestId('funnel-signed-up-count')).toHaveTextContent('4')
    expect(screen.getByTestId('funnel-connected-count')).toHaveTextContent('2')
    expect(screen.getByTestId('funnel-first-turn-count')).toHaveTextContent('1')
    expect(screen.getByTestId('funnel-returned-count')).toHaveTextContent('1')
  })

  it('lists one row per account, with its first-week message count', async () => {
    renderView()

    const rows = await screen.findAllByTestId('new-user-row')
    expect(rows).toHaveLength(2)
    expect(within(rows[0]).getByText('Ana')).toBeInTheDocument()
    expect(within(rows[0]).getByText('ana@example.com')).toBeInTheDocument()
    expect(within(rows[0]).getByTestId('turns-first-week')).toHaveTextContent('5')
    // No display name: the address stands in for it.
    expect(within(rows[1]).getByText('ben@example.com')).toBeInTheDocument()
    expect(within(rows[1]).getByTestId('turns-first-week')).toHaveTextContent('0')
  })

  it('reloads for the window picked', async () => {
    const user = userEvent.setup()
    renderView()
    await screen.findByTestId('new-users-funnel')

    await user.click(screen.getByTestId('new-users-window-7'))

    await waitFor(() => expect(adminApi.getNewUsers).toHaveBeenCalledWith(7))
    expect(screen.getByTestId('new-users-window-7')).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByTestId('new-users-window-30')).toHaveAttribute('aria-pressed', 'false')
  })

  it('says so when the window holds no account', async () => {
    vi.mocked(adminApi.getNewUsers).mockResolvedValue({
      ...COHORT,
      funnel: { ...COHORT.funnel, signed_up: 0 },
      users: [],
    })
    renderView()

    expect(await screen.findByTestId('new-users-empty')).toBeInTheDocument()
    expect(screen.queryByTestId('new-users-table')).not.toBeInTheDocument()
  })

  it('flags a truncated window', async () => {
    vi.mocked(adminApi.getNewUsers).mockResolvedValue({ ...COHORT, truncated: true })
    renderView()

    expect(await screen.findByTestId('new-users-truncated')).toBeInTheDocument()
  })

  it('offers a retry when the report fails to load', async () => {
    vi.mocked(adminApi.getNewUsers).mockRejectedValue(new Error('boom'))
    renderView()

    expect(await screen.findByTestId('new-users-error')).toBeInTheDocument()
  })
})

describe('sinceSignup', () => {
  const signup = '2026-11-01T09:00:00Z'

  it('reads minutes under an hour', () => {
    expect(sinceSignup(signup, '2026-11-01T09:25:00Z')).toBe('+25 min')
  })

  it('reads hours under two days', () => {
    expect(sinceSignup(signup, '2026-11-02T21:00:00Z')).toBe('+36 h')
  })

  it('reads days after that', () => {
    expect(sinceSignup(signup, '2026-11-04T09:00:00Z')).toBe('+3 d')
  })

  it('never reads a negative duration', () => {
    expect(sinceSignup(signup, '2026-11-01T08:00:00Z')).toBe('+0 min')
  })
})

describe('percentOf', () => {
  it('rounds to a whole percentage', () => {
    expect(percentOf(1, 3)).toBe(33)
    expect(percentOf(2, 2)).toBe(100)
  })

  it('has no percentage without a whole', () => {
    expect(percentOf(0, 0)).toBeNull()
  })
})
