// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import PendingApproval from '../PendingApproval'

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

    expect(screen.getByText('Pending')).toBeInTheDocument()
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
})
