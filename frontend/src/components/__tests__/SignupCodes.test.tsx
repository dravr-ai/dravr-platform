// ABOUTME: Tests the admin console's signup-code view: the listing, minting with its one-time reveal, revoking
// ABOUTME: Asserts what the mint form sends, that the code is shown once with a copy button, and revoke's kind/id (carnet#1128)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import SignupCodes from '../SignupCodes'
import type { SignupCodeListing } from '../../services/api/admin'

vi.mock('../../services/api', () => ({
  adminApi: {
    getSignupCodes: vi.fn(),
    mintSignupCode: vi.fn(),
    revokeSignupCode: vi.fn(),
  },
}))

const { adminApi } = await import('../../services/api')

const ADMIN_CODE: SignupCodeListing = {
  kind: 'admin',
  id: 'code-admin-1',
  code_hint: 'JKLM',
  note: 'beta cohort',
  created_by: 'op-1',
  created_by_email: 'admin@example.com',
  group_id: null,
  expires_at: '2026-11-30T00:00:00Z',
  max_uses: 20,
  use_count: 2,
  live: true,
  created_at: '2026-10-20T12:00:00Z',
  redeemed_by: [
    { email: 'ana@example.com', status: 'active', redeemed_at: '2026-10-21T09:00:00Z', activated_at: '2026-10-21T09:00:00Z' },
    { email: 'ben@example.com', status: 'pending', redeemed_at: '2026-10-21T10:00:00Z', activated_at: null },
  ],
}

const COACH_CODE: SignupCodeListing = {
  kind: 'coach',
  id: 'invite-7',
  code_hint: 'X9Q2',
  note: null,
  created_by: 'coach-1',
  created_by_email: null,
  group_id: 'group-1',
  expires_at: null,
  max_uses: null,
  use_count: 0,
  live: false,
  created_at: '2026-10-19T12:00:00Z',
  redeemed_by: [],
}

function renderView() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={queryClient}>
      <SignupCodes />
    </QueryClientProvider>,
  )
}

describe('SignupCodes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(adminApi.getSignupCodes).mockResolvedValue([ADMIN_CODE, COACH_CODE])
  })

  it('lists each code by its hint with kind, uses, liveness, minter, note and who redeemed it', async () => {
    renderView()

    expect(await screen.findByText('…JKLM')).toBeInTheDocument()
    expect(screen.getByText('Signup codes (2)')).toBeInTheDocument()
    expect(screen.getByText('Uses: 2 / 20')).toBeInTheDocument()
    expect(screen.getByText('By: admin@example.com')).toBeInTheDocument()
    expect(screen.getByText('beta cohort')).toBeInTheDocument()
    expect(screen.getByText('ana@example.com')).toBeInTheDocument()
    expect(screen.getByText('ben@example.com')).toBeInTheDocument()
    expect(screen.getByText('pending')).toBeInTheDocument()

    // The coach's code: unlimited, never expires, unattributed, not live.
    expect(screen.getByText('…X9Q2')).toBeInTheDocument()
    expect(screen.getByText('Uses: 0 / ∞')).toBeInTheDocument()
    expect(screen.getByText('Expires: never')).toBeInTheDocument()
    expect(screen.getByText('By: unattributed')).toBeInTheDocument()
    expect(screen.getByText('not live')).toBeInTheDocument()

    // Only a live code can be revoked.
    expect(screen.getByRole('button', { name: 'Revoke code ending JKLM' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Revoke code ending X9Q2' })).not.toBeInTheDocument()
  })

  it('mints with only the fields filled in, and reveals the code once with a copy button', async () => {
    vi.mocked(adminApi.mintSignupCode).mockResolvedValue({
      message: 'Signup code minted. Copy it now: it is not shown again.',
      code: 'BETA-ABCD-EFGH-JKLM',
    })
    const user = userEvent.setup()
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue()
    renderView()
    await screen.findByText('…JKLM')

    await user.type(screen.getByLabelText('Note'), ' beta cohort ')
    await user.type(screen.getByLabelText('Prefix'), 'BETA')
    await user.type(screen.getByLabelText('Max uses'), '25')
    await user.click(screen.getByRole('button', { name: 'Mint code' }))

    await waitFor(() =>
      expect(adminApi.mintSignupCode).toHaveBeenCalledWith({ note: 'beta cohort', prefix: 'BETA', max_uses: 25 }),
    )
    const reveal = await screen.findByTestId('signup-code-minted')
    expect(reveal).toHaveTextContent('BETA-ABCD-EFGH-JKLM')
    expect(reveal).toHaveTextContent("you won't see it again")
    // The list is read again so the new code's row appears.
    await waitFor(() => expect(adminApi.getSignupCodes).toHaveBeenCalledTimes(2))

    await user.click(screen.getByRole('button', { name: 'Copy' }))
    expect(writeText).toHaveBeenCalledWith('BETA-ABCD-EFGH-JKLM')
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Done' }))
    expect(screen.queryByTestId('signup-code-minted')).not.toBeInTheDocument()
  })

  it('refuses out-of-range values before sending anything', async () => {
    renderView()
    await screen.findByText('…JKLM')

    fireEvent.change(screen.getByLabelText('Prefix'), { target: { value: 'BETA-2026!' } })
    fireEvent.change(screen.getByLabelText('Expires in days'), { target: { value: '400' } })

    expect(screen.getByText('1 to 12 letters or digits.')).toBeInTheDocument()
    expect(screen.getByText('A whole number from 1 to 365.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Mint code' })).toBeDisabled()
    expect(adminApi.mintSignupCode).not.toHaveBeenCalled()
  })

  it("shows the server's refusal of a mint", async () => {
    vi.mocked(adminApi.mintSignupCode).mockRejectedValue({
      response: { data: { message: 'prefix must be 1 to 12 letters or digits' } },
    })
    renderView()
    await screen.findByText('…JKLM')

    fireEvent.click(screen.getByRole('button', { name: 'Mint code' }))

    expect(await screen.findByText('prefix must be 1 to 12 letters or digits')).toBeInTheDocument()
    expect(screen.queryByTestId('signup-code-minted')).not.toBeInTheDocument()
  })

  it('revokes a live code by its kind and id', async () => {
    vi.mocked(adminApi.revokeSignupCode).mockResolvedValue({
      message: 'Signup code revoked. Accounts it already approved stay approved.',
      revoked: true,
    })
    renderView()

    fireEvent.click(await screen.findByRole('button', { name: 'Revoke code ending JKLM' }))

    await waitFor(() => expect(adminApi.revokeSignupCode).toHaveBeenCalledWith('admin', 'code-admin-1'))
    expect(
      await screen.findByText('Signup code revoked. Accounts it already approved stay approved.'),
    ).toBeInTheDocument()
  })
})
