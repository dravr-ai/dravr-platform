// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the address-confirmation page: nothing is spent on load, and each outcome leads somewhere
// ABOUTME: An expired link re-sends from its own token; only a link the server never issued asks for the address

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

vi.mock('../../services/api', () => ({
  authApi: { resendVerification: vi.fn() },
  emailConfirmationApi: { confirmEmail: vi.fn(), resendForLink: vi.fn() },
}))

import VerifyEmail from '../VerifyEmail'
import { authApi, emailConfirmationApi } from '../../services/api'

const TOKEN = 'AbCd1234EfGh5678.IjKl9012MnOp3456QrSt7890UvWx1234'

describe('VerifyEmail', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('spends nothing until the button is pressed', async () => {
    vi.mocked(emailConfirmationApi.confirmEmail).mockResolvedValue({ status: 'verified' })
    render(<VerifyEmail token={TOKEN} onContinue={vi.fn()} />)

    expect(screen.getByRole('heading', { name: 'Confirm your email' })).toBeInTheDocument()
    expect(emailConfirmationApi.confirmEmail).not.toHaveBeenCalled()
    expect(screen.queryByLabelText('Email address')).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: 'Confirm my address' }))

    expect(emailConfirmationApi.confirmEmail).toHaveBeenCalledWith(TOKEN)
    expect(await screen.findByRole('heading', { name: 'Email confirmed' })).toBeInTheDocument()
  })

  it('tells a pending account it still waits for review', async () => {
    vi.mocked(emailConfirmationApi.confirmEmail).mockResolvedValue({ status: 'verified_pending' })
    render(<VerifyEmail token={TOKEN} onContinue={vi.fn()} />)

    await userEvent.click(screen.getByRole('button', { name: 'Confirm my address' }))

    expect(await screen.findByText(/administrator still needs to approve/)).toBeInTheDocument()
  })

  it('re-sends an expired link from its token, without asking for the address', async () => {
    vi.mocked(emailConfirmationApi.confirmEmail).mockResolvedValue({ status: 'expired' })
    vi.mocked(emailConfirmationApi.resendForLink).mockResolvedValue({ message: 'ok' })
    render(<VerifyEmail token={TOKEN} onContinue={vi.fn()} />)

    await userEvent.click(screen.getByRole('button', { name: 'Confirm my address' }))
    expect(await screen.findByRole('heading', { name: 'That link has expired' })).toBeInTheDocument()
    expect(screen.queryByLabelText('Email address')).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: 'Send a new link' }))

    expect(emailConfirmationApi.resendForLink).toHaveBeenCalledWith(TOKEN)
    expect(authApi.resendVerification).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.getByRole('status')).toBeInTheDocument())
  })

  it('asks for the address only when the link carries no token it can use', async () => {
    vi.mocked(authApi.resendVerification).mockResolvedValue({ message: 'ok' })
    render(<VerifyEmail token={null} onContinue={vi.fn()} />)

    expect(screen.getByRole('heading', { name: "This link doesn't work" })).toBeInTheDocument()
    await userEvent.type(screen.getByLabelText('Email address'), 'me@example.com')
    await userEvent.click(screen.getByRole('button', { name: 'Send a new link' }))

    expect(authApi.resendVerification).toHaveBeenCalledWith('me@example.com')
  })

  it('offers another press when the server fails', async () => {
    vi.mocked(emailConfirmationApi.confirmEmail)
      .mockRejectedValueOnce(new Error('503'))
      .mockResolvedValueOnce({ status: 'verified' })
    render(<VerifyEmail token={TOKEN} onContinue={vi.fn()} />)

    await userEvent.click(screen.getByRole('button', { name: 'Confirm my address' }))
    await userEvent.click(await screen.findByRole('button', { name: 'Try Again' }))

    expect(await screen.findByRole('heading', { name: 'Email confirmed' })).toBeInTheDocument()
    expect(emailConfirmationApi.confirmEmail).toHaveBeenCalledTimes(2)
  })
})
