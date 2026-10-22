// ABOUTME: Tests the group invite panel's signup-code option: shown only to a coach with coach access, on a member invite
// ABOUTME: Asserts the toggle sends approves_signup with bounded uses and expiry, and the badge marks such invites (carnet#1128)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { GroupInvite } from '@pierre/shared-types'
import InviteManager from '../InviteManager'
import { ToastProvider } from '../../ui'

const createInvite = vi.fn()
let invites: GroupInvite[] = []
let authUser: { id: string; manages_roster?: boolean } = { id: 'coach-1', manages_roster: true }

vi.mock('../../../hooks/useGroups', () => ({
  useGroupInvites: () => ({ invites, isLoading: false }),
  useCreateInvite: () => ({ createInvite, isPending: false }),
  useDeactivateInvite: () => ({ deactivateInvite: vi.fn(), isPending: false }),
}))

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: authUser }),
}))

function invite(overrides: Partial<GroupInvite> = {}): GroupInvite {
  return {
    id: 'invite-1',
    group_id: 'group-1',
    tenant_id: 'tenant-1',
    code: 'MRT2026X',
    kind: 'member',
    created_by: 'coach-1',
    expires_at: '2027-06-01T00:00:00Z',
    max_uses: 10,
    use_count: 3,
    is_active: true,
    approves_signup: false,
    created_at: '2026-10-15T10:00:00Z',
    ...overrides,
  } as GroupInvite
}

function renderManager() {
  return render(
    <ToastProvider>
      <InviteManager groupId="group-1" currentUserRole="owner" />
    </ToastProvider>,
  )
}

const TOGGLE = 'Also approves new accounts'

describe('InviteManager signup codes', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    invites = []
    authUser = { id: 'coach-1', manages_roster: true }
    createInvite.mockResolvedValue(invite())
  })

  it('sends approves_signup with bounded uses and expiry when the toggle is on', async () => {
    const user = userEvent.setup()
    renderManager()

    await user.click(screen.getByRole('button', { name: /New invite/i }))
    // An unlimited choice is replaced the moment the toggle bounds the code.
    await user.selectOptions(screen.getByLabelText('Max Uses'), '0')
    await user.selectOptions(screen.getByLabelText('Expires After'), '0')
    await user.click(screen.getByRole('checkbox', { name: new RegExp(TOGGLE) }))

    const maxUses = screen.getByLabelText<HTMLSelectElement>('Max Uses')
    const expiry = screen.getByLabelText<HTMLSelectElement>('Expires After')
    expect(Array.from(maxUses.options).map((o) => o.value)).not.toContain('0')
    expect(Array.from(expiry.options).map((o) => o.value)).not.toContain('0')

    await user.selectOptions(maxUses, '25')
    await user.selectOptions(expiry, '30')
    await user.click(screen.getByRole('button', { name: 'Create' }))

    await waitFor(() =>
      expect(createInvite).toHaveBeenCalledWith({ approves_signup: true, max_uses: 25, expires_in_days: 30 }),
    )
  })

  it('falls back to bounded defaults when an unlimited choice was picked before the toggle', async () => {
    const user = userEvent.setup()
    renderManager()

    await user.click(screen.getByRole('button', { name: /New invite/i }))
    await user.selectOptions(screen.getByLabelText('Max Uses'), '0')
    await user.selectOptions(screen.getByLabelText('Expires After'), '0')
    await user.click(screen.getByRole('checkbox', { name: new RegExp(TOGGLE) }))
    await user.click(screen.getByRole('button', { name: 'Create' }))

    await waitFor(() =>
      expect(createInvite).toHaveBeenCalledWith({ approves_signup: true, max_uses: 10, expires_in_days: 7 }),
    )
  })

  it('sends no approves_signup when the toggle is left off', async () => {
    const user = userEvent.setup()
    renderManager()

    await user.click(screen.getByRole('button', { name: /New invite/i }))
    await user.click(screen.getByRole('button', { name: 'Create' }))

    await waitFor(() => expect(createInvite).toHaveBeenCalledWith({ max_uses: 10, expires_in_days: 7 }))
  })

  it('offers no toggle to a coach without coach access, nor on a coach invite', async () => {
    authUser = { id: 'owner-1', manages_roster: false }
    const user = userEvent.setup()
    const { unmount } = renderManager()

    await user.click(screen.getByRole('button', { name: /New invite/i }))
    expect(screen.queryByRole('checkbox', { name: new RegExp(TOGGLE) })).not.toBeInTheDocument()
    unmount()

    authUser = { id: 'coach-1', manages_roster: true }
    renderManager()
    await user.click(screen.getByRole('button', { name: /New invite/i }))
    await user.selectOptions(screen.getByLabelText('Invite Type'), 'coach')
    expect(screen.queryByRole('checkbox', { name: new RegExp(TOGGLE) })).not.toBeInTheDocument()
  })

  it('marks an invite that approves signups with the signup-code badge', () => {
    invites = [
      invite({ id: 'invite-1', code: 'PLAIN001' }),
      invite({ id: 'invite-2', code: 'SIGNUP02', approves_signup: true }),
    ]
    renderManager()

    const badges = screen.getAllByTestId('invite-signup-code-badge')
    expect(badges).toHaveLength(1)
    expect(badges[0]).toHaveTextContent('Signup code')
    expect(badges[0].closest('div')).toHaveTextContent('SIGNUP02')
  })
})
