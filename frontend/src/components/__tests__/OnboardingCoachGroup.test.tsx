// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingCoachGroup — the coach names a group and leaves with its invite link
// ABOUTME: Pins the create → thread → 30-day invite sequence, the access-pending state, retry and skip

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import OnboardingCoachGroup from '../OnboardingCoachGroup';

const { createGroupMock, createInviteMock, createConversationMock } = vi.hoisted(() => ({
  createGroupMock: vi.fn(),
  createInviteMock: vi.fn(),
  createConversationMock: vi.fn(),
}));

vi.mock('../../services/api', () => ({
  groupsApi: { createGroup: createGroupMock, createInvite: createInviteMock },
  chatApi: { createConversation: createConversationMock },
}));

vi.mock('qrcode', () => ({
  default: { toDataURL: vi.fn().mockResolvedValue('data:image/png;base64,QR') },
}));

const group = (coachUserId: string | null) => ({
  id: 'g-1',
  tenant_id: 't-1',
  name: 'Les Rouleurs',
  description: null,
  agent_id: 'a-1',
  owner_id: 'u-1',
  coach_user_id: coachUserId,
});

function renderStep() {
  const onComplete = vi.fn();
  render(<OnboardingCoachGroup onComplete={onComplete} />);
  return { onComplete };
}

async function nameAndCreate() {
  await userEvent.type(screen.getByTestId('onboarding-group-name'), 'Les Rouleurs');
  await userEvent.click(screen.getByRole('button', { name: 'Create the group' }));
}

describe('OnboardingCoachGroup', () => {
  beforeEach(() => {
    createGroupMock.mockReset().mockResolvedValue(group('u-1'));
    createConversationMock.mockReset().mockResolvedValue({ id: 'c-1' });
    createInviteMock.mockReset().mockResolvedValue({ code: 'ABCD2345' });
  });

  it('creates the group as its coach, opens its thread and issues a 30-day invite', async () => {
    renderStep();
    await nameAndCreate();

    await waitFor(() =>
      expect(createGroupMock).toHaveBeenCalledWith({ name: 'Les Rouleurs', coach_is_me: true }),
    );
    expect(createConversationMock).toHaveBeenCalledWith({ group_id: 'g-1', agent_id: 'a-1' });
    expect(createInviteMock).toHaveBeenCalledWith('g-1', { expires_in_days: 30 });

    expect(await screen.findByTestId('onboarding-group-link')).toHaveTextContent(
      '/groups/join/ABCD2345',
    );
    expect(await screen.findByTestId('onboarding-group-qr')).toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-group-access-pending')).not.toBeInTheDocument();
  });

  it('says coach access is pending when the group comes back without a coach', async () => {
    createGroupMock.mockResolvedValue(group(null));
    renderStep();
    await nameAndCreate();

    expect(await screen.findByTestId('onboarding-group-access-pending')).toHaveTextContent(
      'Coach access pending',
    );
  });

  it('completes the step from the share screen', async () => {
    const { onComplete } = renderStep();
    await nameAndCreate();
    await userEvent.click(await screen.findByRole('button', { name: 'Go to my group' }));
    expect(onComplete).toHaveBeenCalledWith('complete');
  });

  it('keeps the coach on the step after a failure and retries without a second group', async () => {
    createInviteMock.mockRejectedValueOnce(new Error('network'));
    renderStep();
    await nameAndCreate();

    expect(await screen.findByText("We couldn't create the group. Try again.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Create the group' }));

    expect(await screen.findByTestId('onboarding-group-link')).toBeInTheDocument();
    expect(createGroupMock).toHaveBeenCalledTimes(1);
  });

  it('puts the step off with Later and creates nothing', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByRole('button', { name: 'Later' }));
    expect(onComplete).toHaveBeenCalledWith('skipped');
    expect(createGroupMock).not.toHaveBeenCalled();
  });

  it('cannot create a group without a name', () => {
    renderStep();
    expect(screen.getByRole('button', { name: 'Create the group' })).toBeDisabled();
  });
});
