// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import PasswordResetModal from '../PasswordResetModal'

vi.mock('../../services/api', () => ({
  adminApi: {
    resetUserPassword: vi.fn(),
  },
}))

import { adminApi } from '../../services/api'

const mockUser = {
  id: 'user-1',
  email: 'target@example.com',
  display_name: 'Target User',
  user_status: 'active' as const,
  role: 'user',
}

function renderWithProviders(component: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })
  return render(
    <QueryClientProvider client={queryClient}>
      {component}
    </QueryClientProvider>
  )
}

describe('PasswordResetModal', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should not render when isOpen is false', () => {
    renderWithProviders(
      <PasswordResetModal user={mockUser} isOpen={false} onClose={vi.fn()} />
    )

    expect(screen.queryByText('Reset User Password')).not.toBeInTheDocument()
  })

  it('should not render when user is null', () => {
    renderWithProviders(
      <PasswordResetModal user={null} isOpen={true} onClose={vi.fn()} />
    )

    expect(screen.queryByText('Reset User Password')).not.toBeInTheDocument()
  })

  it('should render modal when open with user', () => {
    renderWithProviders(
      <PasswordResetModal user={mockUser} isOpen={true} onClose={vi.fn()} />
    )

    expect(screen.getByText('Reset User Password')).toBeInTheDocument()
    expect(screen.getByText('Target User')).toBeInTheDocument()
    expect(screen.getByText('target@example.com')).toBeInTheDocument()
    expect(screen.getByText('active')).toBeInTheDocument()
  })

  it('should show warning message', () => {
    renderWithProviders(
      <PasswordResetModal user={mockUser} isOpen={true} onClose={vi.fn()} />
    )

    expect(screen.getByText(/generate a temporary password/)).toBeInTheDocument()
  })

  it('should show Reset Password and Cancel buttons', () => {
    renderWithProviders(
      <PasswordResetModal user={mockUser} isOpen={true} onClose={vi.fn()} />
    )

    expect(screen.getByText('Reset Password')).toBeInTheDocument()
    expect(screen.getByText('Cancel')).toBeInTheDocument()
  })

  it('should call onClose when Cancel is clicked', async () => {
    const onClose = vi.fn()
    const user = userEvent.setup()

    renderWithProviders(
      <PasswordResetModal user={mockUser} isOpen={true} onClose={onClose} />
    )

    await user.click(screen.getByText('Cancel'))
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('should call onClose when close button is clicked', async () => {
    const onClose = vi.fn()
    const user = userEvent.setup()

    renderWithProviders(
      <PasswordResetModal user={mockUser} isOpen={true} onClose={onClose} />
    )

    await user.click(screen.getByLabelText('Close modal'))
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('should show temporary password after successful reset', async () => {
    const user = userEvent.setup()
    vi.mocked(adminApi.resetUserPassword).mockResolvedValue({
      success: true,
      temporary_password: 'TempPass123!',
      expires_at: '2026-02-21T00:00:00Z',
      user_email: 'target@example.com',
    })

    renderWithProviders(
      <PasswordResetModal user={mockUser} isOpen={true} onClose={vi.fn()} />
    )

    await user.click(screen.getByText('Reset Password'))

    await waitFor(() => {
      expect(screen.getByText('Password Reset Successful')).toBeInTheDocument()
    })

    expect(screen.getByText('TempPass123!')).toBeInTheDocument()
    expect(screen.getByText('Done')).toBeInTheDocument()
  })

  it('should show Unnamed User when display_name is missing', () => {
    const userWithoutName = { ...mockUser, display_name: undefined }

    renderWithProviders(
      <PasswordResetModal user={userWithoutName} isOpen={true} onClose={vi.fn()} />
    )

    expect(screen.getByText('Unnamed User')).toBeInTheDocument()
  })

  it('should show pending badge for pending users', () => {
    const pendingUser = { ...mockUser, user_status: 'pending' as const }

    renderWithProviders(
      <PasswordResetModal user={pendingUser} isOpen={true} onClose={vi.fn()} />
    )

    expect(screen.getByText('pending')).toBeInTheDocument()
  })
})
