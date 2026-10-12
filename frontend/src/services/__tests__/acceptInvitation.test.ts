// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the request the web app sends when an invitee accepts a mailed invitation (carnet#43)
// ABOUTME: The endpoint, the token and chosen password in the body, and a blank name sent as null rather than an empty string

import { describe, it, expect, vi, beforeEach } from 'vitest'

const { post } = vi.hoisted(() => ({ post: vi.fn() }))

// The shared client's axios carries the CSRF and 401 interceptors; only its
// transport is replaced here, so the module under test is the real one.
vi.mock('../api/client', () => ({ axios: { post } }))

import { invitationsApi } from '../api/invitations'

const TOKEN = 'AbCd1234EfGh5678.IjKl9012MnOp3456QrSt7890UvWx1234'
const REPLY = {
  user_id: 'user-1',
  user_status: 'active',
  display_name: 'Coach Phil',
  message: 'Your account is ready. Sign in with your email and the password you chose.',
}

describe('invitationsApi.acceptInvitation', () => {
  beforeEach(() => {
    post.mockReset()
    post.mockResolvedValue({ data: REPLY })
  })

  it('posts the token and the chosen password to the acceptance endpoint', async () => {
    const result = await invitationsApi.acceptInvitation(TOKEN, 'chosenByTheInvitee1', 'Coach Phil')

    expect(post).toHaveBeenCalledTimes(1)
    expect(post).toHaveBeenCalledWith('/api/auth/accept-invitation', {
      token: TOKEN,
      password: 'chosenByTheInvitee1',
      display_name: 'Coach Phil',
    })
    expect(result).toEqual(REPLY)
  })

  it('sends a missing display name as null, never an empty string', async () => {
    await invitationsApi.acceptInvitation(TOKEN, 'chosenByTheInvitee1')
    await invitationsApi.acceptInvitation(TOKEN, 'chosenByTheInvitee1', '')

    for (const call of post.mock.calls) {
      expect(call[1]).toEqual({ token: TOKEN, password: 'chosenByTheInvitee1', display_name: null })
    }
    expect(post).toHaveBeenCalledTimes(2)
  })

  it('lets a refusal reach the page, which turns a 404 into the expired-link screen', async () => {
    const refusal = { response: { status: 404, data: { message: 'The requested resource was not found' } } }
    post.mockRejectedValueOnce(refusal)

    await expect(invitationsApi.acceptInvitation(TOKEN, 'chosenByTheInvitee1')).rejects.toBe(refusal)
  })
})
