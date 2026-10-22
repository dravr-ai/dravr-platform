// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

vi.mock('../../services/api', () => ({
  authApi: { resendVerification: vi.fn() },
  userApi: { clearImportTombstone: vi.fn(), redeemSignupCode: vi.fn() },
}))

import PendingApproval from '../PendingApproval'
import { userApi } from '../../services/api'

const mockLogout = vi.fn()

vi.mock('../../hooks/useAuth', () => ({
  useAuth: vi.fn(),
}))

import { useAuth } from '../../hooks/useAuth'

describe('PendingApproval', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should render pending approval title', () => {
    vi.mocked(useAuth).mockReturnValue({
      user: { id: '1', email: 'pending@example.com', display_name: 'Pending User' },
      logout: mockLogout,
    } as ReturnType<typeof useAuth>)

    render(<PendingApproval />)

    expect(screen.getByText('Account Pending Approval')).toBeInTheDocument()
  })

  it('should show user email', () => {
    vi.mocked(useAuth).mockReturnValue({
      user: { id: '1', email: 'pending@example.com', display_name: 'Pending User' },
      logout: mockLogout,
    } as ReturnType<typeof useAuth>)

    render(<PendingApproval />)

    expect(screen.getByText('pending@example.com')).toBeInTheDocument()
  })

  it('should show user display name', () => {
    vi.mocked(useAuth).mockReturnValue({
      user: { id: '1', email: 'pending@example.com', display_name: 'Pending User' },
      logout: mockLogout,
    } as ReturnType<typeof useAuth>)

    render(<PendingApproval />)

    expect(screen.getByText('Pending User')).toBeInTheDocument()
  })

  it('should show Pending badge', () => {
    vi.mocked(useAuth).mockReturnValue({
      user: { id: '1', email: 'pending@example.com' },
      logout: mockLogout,
    } as ReturnType<typeof useAuth>)

    render(<PendingApproval />)

    expect(screen.getByText('Awaiting review')).toBeInTheDocument()
  })

  it('should show what happens next section', () => {
    vi.mocked(useAuth).mockReturnValue({
      user: { id: '1', email: 'pending@example.com' },
      logout: mockLogout,
    } as ReturnType<typeof useAuth>)

    render(<PendingApproval />)

    expect(screen.getByText('What happens next?')).toBeInTheDocument()
    expect(screen.getByText(/administrator will review/)).toBeInTheDocument()
    expect(screen.getByText(/receive an email when approved/)).toBeInTheDocument()
  })

  it('should call logout when Sign Out is clicked', async () => {
    const user = userEvent.setup()

    vi.mocked(useAuth).mockReturnValue({
      user: { id: '1', email: 'pending@example.com' },
      logout: mockLogout,
    } as ReturnType<typeof useAuth>)

    render(<PendingApproval />)

    await user.click(screen.getByText('Sign Out'))
    expect(mockLogout).toHaveBeenCalledTimes(1)
  })

  it('should show informational message about account creation', () => {
    vi.mocked(useAuth).mockReturnValue({
      user: { id: '1', email: 'pending@example.com' },
      logout: mockLogout,
    } as ReturnType<typeof useAuth>)

    render(<PendingApproval />)

    expect(screen.getByText(/account has been created successfully/)).toBeInTheDocument()
  })

  describe('an email a deleted account left behind', () => {
    const tombstone = { tombstone_id: 'tomb-1', deleted_at: '2026-10-20T08:00:00+00:00' }

    it('says why the account waits and offers to start fresh once the address is proven', async () => {
      const mockUpdateUser = vi.fn()
      vi.mocked(userApi.clearImportTombstone).mockResolvedValue({ cleared: [], user_status: 'active' })
      vi.mocked(useAuth).mockReturnValue({
        user: { id: '1', email: 'back@example.com', email_verified: true, email_tombstone: tombstone },
        logout: mockLogout,
        updateUser: mockUpdateUser,
      } as unknown as ReturnType<typeof useAuth>)
      const user = userEvent.setup()

      render(<PendingApproval />)

      expect(
        screen.getByText(
          'This email belonged to a Dravr account deleted on Oct 20, 2026. Confirm you want to start fresh with it to continue.',
        ),
      ).toBeInTheDocument()
      // Nobody reviews it, so the review steps are not promised.
      expect(screen.queryByText(/administrator will review/)).not.toBeInTheDocument()

      await user.click(screen.getByRole('button', { name: 'Start fresh' }))

      expect(userApi.clearImportTombstone).toHaveBeenCalledWith('tomb-1')
      await waitFor(() =>
        expect(mockUpdateUser).toHaveBeenCalledWith({ user_status: 'active', email_tombstone: undefined }),
      )
    })

    it('asks for the address to be confirmed first, and offers no clear before it is', () => {
      vi.mocked(useAuth).mockReturnValue({
        user: { id: '1', email: 'back@example.com', email_verified: false, email_tombstone: tombstone },
        logout: mockLogout,
        updateUser: vi.fn(),
      } as unknown as ReturnType<typeof useAuth>)

      render(<PendingApproval />)

      expect(screen.getByText('Confirm your email address first, then start fresh.')).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Start fresh' })).not.toBeInTheDocument()
      // The server refuses a code beside a deleted account's address, so none is offered.
      expect(screen.queryByTestId('pending-signup-code')).not.toBeInTheDocument()
    })
  })
  describe('a signup code', () => {
    function renderPending(emailVerified: boolean | undefined = true) {
      const mockUpdateUser = vi.fn()
      vi.mocked(useAuth).mockReturnValue({
        user: { id: '1', email: 'pending@example.com', email_verified: emailVerified },
        logout: mockLogout,
        updateUser: mockUpdateUser,
      } as unknown as ReturnType<typeof useAuth>)
      render(<PendingApproval />)
      return { mockUpdateUser }
    }

    async function enterCode(user: ReturnType<typeof userEvent.setup>, code: string) {
      await user.click(screen.getByRole('button', { name: 'Have a signup code?' }))
      await user.type(screen.getByLabelText('Signup code'), code)
      await user.click(screen.getByRole('button', { name: 'Use code' }))
    }

    it('activates the account at once: the user is refreshed to active', async () => {
      vi.mocked(userApi.redeemSignupCode).mockResolvedValue({ user_status: 'active', group_joined: null })
      const user = userEvent.setup()
      const { mockUpdateUser } = renderPending()

      await enterCode(user, ' BETA-ABCD-EFGH-JKLM ')

      expect(userApi.redeemSignupCode).toHaveBeenCalledWith('BETA-ABCD-EFGH-JKLM')
      await waitFor(() => expect(mockUpdateUser).toHaveBeenCalledWith({ user_status: 'active' }))
    })

    it('says the confirmation link is what remains when the code leaves the account pending', async () => {
      vi.mocked(userApi.redeemSignupCode).mockResolvedValue({ user_status: 'pending', group_joined: null })
      const user = userEvent.setup()
      const { mockUpdateUser } = renderPending(false)

      await enterCode(user, 'BETA-ABCD-EFGH-JKLM')

      expect(
        await screen.findByText('Code accepted. Confirm your email address to activate your account.'),
      ).toBeInTheDocument()
      expect(screen.queryByText("You've also joined the group this code came from.")).not.toBeInTheDocument()
      expect(screen.queryByLabelText('Signup code')).not.toBeInTheDocument()
      expect(mockUpdateUser).not.toHaveBeenCalled()
    })

    it("also says a coach's code joined the coach's group", async () => {
      vi.mocked(userApi.redeemSignupCode).mockResolvedValue({ user_status: 'pending', group_joined: true })
      const user = userEvent.setup()
      renderPending(false)

      await enterCode(user, 'COACH-1234')

      expect(await screen.findByText("You've also joined the group this code came from.")).toBeInTheDocument()
    })

    it("shows the server's refusal inline and keeps the field to retry", async () => {
      vi.mocked(userApi.redeemSignupCode).mockRejectedValue({
        isAxiosError: true,
        response: { status: 400, data: { message: "This code isn't valid. Check it and try again." } },
      })
      const user = userEvent.setup()
      const { mockUpdateUser } = renderPending()

      await enterCode(user, 'NOPE')

      expect(await screen.findByRole('alert')).toHaveTextContent("This code isn't valid. Check it and try again.")
      expect(screen.getByLabelText('Signup code')).toHaveValue('NOPE')
      expect(mockUpdateUser).not.toHaveBeenCalled()
    })
  })
})
