// ABOUTME: Tests the group invite panel's signup-code option: shown only to a coach with coach access, on a member invite
// ABOUTME: Also pins the catalogue-worded choices, the quiet deactivate behind the confirm, and that only live links are listed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { GroupInvite } from '@pierre/shared-types'
import { i18n } from '@pierre/i18n'
import InviteManager from '../InviteManager'
import { ToastProvider } from '../../ui'

const createInvite = vi.fn()
const deactivateInvite = vi.fn()
let invites: GroupInvite[] = []
let authUser: { id: string; manages_roster?: boolean } = { id: 'coach-1', manages_roster: true }

vi.mock('../../../hooks/useGroups', () => ({
  useGroupInvites: () => ({ invites, isLoading: false }),
  useCreateInvite: () => ({ createInvite, isPending: false }),
  useDeactivateInvite: () => ({ deactivateInvite, isPending: false }),
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
    await user.selectOptions(screen.getByLabelText('Number of uses'), '0')
    await user.selectOptions(screen.getByLabelText('Expires after'), '0')
    await user.click(screen.getByRole('checkbox', { name: new RegExp(TOGGLE) }))

    const maxUses = screen.getByLabelText<HTMLSelectElement>('Number of uses')
    const expiry = screen.getByLabelText<HTMLSelectElement>('Expires after')
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
    await user.selectOptions(screen.getByLabelText('Number of uses'), '0')
    await user.selectOptions(screen.getByLabelText('Expires after'), '0')
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
    await user.selectOptions(screen.getByLabelText('Invite type'), 'coach')
    expect(screen.queryByRole('checkbox', { name: new RegExp(TOGGLE) })).not.toBeInTheDocument()
  })

  it('words every expiry and use limit from the catalogue, singular and plural, in French too', async () => {
    const user = userEvent.setup()
    await i18n.changeLanguage('fr')
    try {
      renderManager()
      await user.click(screen.getByRole('button', { name: /Nouveau lien d’invitation/ }))
      const uses = Array.from(screen.getByLabelText<HTMLSelectElement>('Nombre d’utilisations').options).map((o) => o.text)
      const expiry = Array.from(screen.getByLabelText<HTMLSelectElement>('Expire après').options).map((o) => o.text)
      expect(uses).toEqual(['1 utilisation', '5 utilisations', '10 utilisations', '25 utilisations', 'Illimité'])
      expect(expiry).toEqual(['1 jour', '3 jours', '7 jours', '14 jours', '30 jours', 'N’expire jamais'])
    } finally {
      await i18n.changeLanguage('en')
    }
  })

  it('deactivates a link from its quiet menu, and fills the danger only in the confirm', async () => {
    invites = [invite({ id: 'invite-1', code: 'PLAIN001' })]
    const user = userEvent.setup()
    renderManager()

    // No filled red on the row: the only way to deactivate is the ⋯ menu.
    expect(document.querySelector('.btn-danger')).toBeNull()
    await user.click(screen.getByRole('button', { name: 'More actions for invite PLAIN001' }))
    await user.click(screen.getByRole('menuitem', { name: 'Deactivate link…' }))

    const confirm = await screen.findByRole('dialog')
    expect(confirm).toHaveTextContent('Deactivate this link?')
    expect(confirm).toHaveTextContent('This invite link stops working right away.')
    await user.click(within(confirm).getByRole('button', { name: 'Deactivate' }))
    await waitFor(() => expect(deactivateInvite).toHaveBeenCalledWith('invite-1'))
  })

  it('lists only the links that still let someone in', () => {
    invites = [
      invite({ id: 'live', code: 'LIVE0001' }),
      invite({ id: 'used-up', code: 'USED0002', max_uses: 3, use_count: 3 }),
      invite({ id: 'off', code: 'OFF00003', is_active: false }),
    ]
    renderManager()

    expect(screen.getByText('LIVE0001')).toBeInTheDocument()
    expect(screen.queryByText('USED0002')).toBeNull()
    expect(screen.queryByText('OFF00003')).toBeNull()
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
