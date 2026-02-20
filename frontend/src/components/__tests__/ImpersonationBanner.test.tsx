// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import ImpersonationBanner from '../ImpersonationBanner'

const mockEndImpersonation = vi.fn()

vi.mock('../../hooks/useAuth', () => ({
  useAuth: vi.fn(),
}))

// Import the mock so we can change return values per test
import { useAuth } from '../../hooks/useAuth'

describe('ImpersonationBanner', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockEndImpersonation.mockResolvedValue(undefined)
  })

  it('should not render when not impersonating', () => {
    vi.mocked(useAuth).mockReturnValue({
      impersonation: {
        isImpersonating: false,
        targetUser: null,
        sessionId: null,
        originalUser: null,
      },
      endImpersonation: mockEndImpersonation,
    } as ReturnType<typeof useAuth>)

    const { container } = render(<ImpersonationBanner />)
    expect(container.firstChild).toBeNull()
  })

  it('should render when impersonating a user', () => {
    vi.mocked(useAuth).mockReturnValue({
      impersonation: {
        isImpersonating: true,
        targetUser: {
          id: 'user-2',
          email: 'target@example.com',
          display_name: 'Target User',
          role: 'user',
        },
        sessionId: 'session-1',
        originalUser: null,
      },
      endImpersonation: mockEndImpersonation,
    } as ReturnType<typeof useAuth>)

    render(<ImpersonationBanner />)

    expect(screen.getByText(/You are impersonating/)).toBeInTheDocument()
    expect(screen.getByText('Target User')).toBeInTheDocument()
    expect(screen.getByText('(target@example.com)')).toBeInTheDocument()
    expect(screen.getByText(/Role: user/)).toBeInTheDocument()
  })

  it('should show email when display_name is not set', () => {
    vi.mocked(useAuth).mockReturnValue({
      impersonation: {
        isImpersonating: true,
        targetUser: {
          id: 'user-2',
          email: 'target@example.com',
          role: 'user',
        },
        sessionId: 'session-1',
        originalUser: null,
      },
      endImpersonation: mockEndImpersonation,
    } as ReturnType<typeof useAuth>)

    render(<ImpersonationBanner />)

    expect(screen.getByText('target@example.com')).toBeInTheDocument()
  })

  it('should show End Impersonation button', () => {
    vi.mocked(useAuth).mockReturnValue({
      impersonation: {
        isImpersonating: true,
        targetUser: {
          id: 'user-2',
          email: 'target@example.com',
          display_name: 'Target User',
          role: 'user',
        },
        sessionId: 'session-1',
        originalUser: null,
      },
      endImpersonation: mockEndImpersonation,
    } as ReturnType<typeof useAuth>)

    render(<ImpersonationBanner />)

    expect(screen.getByText('End Impersonation')).toBeInTheDocument()
  })

  it('should call endImpersonation when button is clicked', async () => {
    const user = userEvent.setup()

    vi.mocked(useAuth).mockReturnValue({
      impersonation: {
        isImpersonating: true,
        targetUser: {
          id: 'user-2',
          email: 'target@example.com',
          display_name: 'Target User',
          role: 'user',
        },
        sessionId: 'session-1',
        originalUser: null,
      },
      endImpersonation: mockEndImpersonation,
    } as ReturnType<typeof useAuth>)

    render(<ImpersonationBanner />)

    await user.click(screen.getByText('End Impersonation'))
    expect(mockEndImpersonation).toHaveBeenCalledTimes(1)
  })

  it('should handle endImpersonation error gracefully', async () => {
    const user = userEvent.setup()
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
    mockEndImpersonation.mockRejectedValue(new Error('Network error'))

    vi.mocked(useAuth).mockReturnValue({
      impersonation: {
        isImpersonating: true,
        targetUser: {
          id: 'user-2',
          email: 'target@example.com',
          display_name: 'Target User',
          role: 'user',
        },
        sessionId: 'session-1',
        originalUser: null,
      },
      endImpersonation: mockEndImpersonation,
    } as ReturnType<typeof useAuth>)

    render(<ImpersonationBanner />)
    await user.click(screen.getByText('End Impersonation'))

    expect(consoleError).toHaveBeenCalledWith('Failed to end impersonation:', expect.any(Error))
    consoleError.mockRestore()
  })
})
