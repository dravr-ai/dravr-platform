// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Every hand-drawn admin and agent dialog is a layer: Escape closes it and focus goes back to its opener
// ABOUTME: And a dialog opened over one of them closes first — the review drawer under its rejection modal

import { useState, type ReactNode } from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { User } from '../../types/api';
import CoachRejectionModal from '../CoachRejectionModal';
import CoachReviewDrawer from '../CoachReviewDrawer';
import PasswordResetModal from '../PasswordResetModal';
import UserApprovalModal from '../UserApprovalModal';
import CoachFormModal from '../discover/CoachFormModal';
import { DEFAULT_COACH_FORM_DATA } from '../discover/coachForm';
import { adminApi } from '../../services/api';

vi.mock('../../services/api', () => ({
  adminApi: {
    approveStoreCoach: vi.fn(),
    rejectStoreCoach: vi.fn(),
    resetUserPassword: vi.fn(),
    approveUser: vi.fn(),
    suspendUser: vi.fn(),
  },
}));

const USER: User = {
  id: 'user-1',
  email: 'target@example.com',
  display_name: 'Target User',
  is_admin: false,
  role: 'user',
  user_status: 'pending',
};

const COACH = {
  id: 'coach-1',
  title: 'Marathon Training Coach',
  description: 'A marathon block.',
  category: 'Training',
  tags: [],
  sample_prompts: [],
  token_count: 1200,
  install_count: 0,
  icon_url: null,
  published_at: null,
  author_id: 'author-123',
  author_email: 'coach@example.com',
  system_prompt: 'You are a marathon coach.',
  created_at: '2024-01-10T00:00:00Z',
  submitted_at: '2024-01-15T10:30:00Z',
  publish_status: 'pending_review',
  package: { artefacts: [], evidence_checked: true },
};

/** An opener, and the dialog it opens; the dialog closes itself. */
function Opener({ dialog }: { dialog: (open: boolean, close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        opener
      </button>
      {dialog(open, () => setOpen(false))}
    </>
  );
}

function renderWithQuery(ui: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

const CASES: Array<{ name: string; title: string; dialog: (open: boolean, close: () => void) => ReactNode }> = [
  {
    name: 'CoachRejectionModal',
    title: 'Reject Agent',
    dialog: (open, close) => <CoachRejectionModal coach={COACH} isOpen={open} onClose={close} onComplete={close} />,
  },
  {
    name: 'CoachReviewDrawer',
    title: 'Review Agent',
    dialog: (open, close) => <CoachReviewDrawer coach={COACH} isOpen={open} onClose={close} />,
  },
  {
    name: 'PasswordResetModal',
    title: 'Reset User Password',
    dialog: (open, close) => <PasswordResetModal user={USER} isOpen={open} onClose={close} />,
  },
  {
    name: 'UserApprovalModal',
    title: 'Approve User',
    dialog: (open, close) => <UserApprovalModal user={USER} isOpen={open} onClose={close} action="approve" />,
  },
  {
    name: 'CoachFormModal',
    title: 'Edit Agent',
    dialog: (open, close) => (
      <CoachFormModal
        isOpen={open}
        formData={{ ...DEFAULT_COACH_FORM_DATA, title: 'Marathon Coach', system_prompt: 'Coach.' }}
        onFormDataChange={vi.fn()}
        onSubmit={vi.fn()}
        onClose={close}
        isSubmitting={false}
        submitError={false}
      />
    ),
  },
];

describe('hand-drawn dialogs on the layer stack', () => {
  it.each(CASES)('$name is a named dialog that Escape closes, handing focus back to its opener', async ({ title, dialog }) => {
    const user = userEvent.setup();
    renderWithQuery(<Opener dialog={dialog} />);
    const opener = screen.getByRole('button', { name: 'opener' });

    await user.click(opener);
    const shown = screen.getByRole('dialog', { name: title });
    // Focus moved in, and the page behind is frozen.
    expect(shown).toContainElement(document.activeElement as HTMLElement);
    expect(document.body.style.overflow).toBe('hidden');

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: title })).not.toBeInTheDocument();
    expect(opener).toHaveFocus();
    expect(document.body.style.overflow).toBe('');
  });

  it('keeps an issued reset token on screen through an Escape; its close button still closes', async () => {
    vi.mocked(adminApi.resetUserPassword).mockResolvedValue({
      user_id: USER.id,
      email: USER.email,
      reset_token: 'reset-token-shown-once',
      expires_in_seconds: 3600,
      reset_by: 'admin-1',
      note: '',
    });
    const user = userEvent.setup();
    const onClose = vi.fn();
    renderWithQuery(<PasswordResetModal user={USER} isOpen onClose={onClose} />);

    await user.click(screen.getByRole('button', { name: 'Reset Password' }));
    expect(await screen.findByText('reset-token-shown-once')).toBeInTheDocument();

    // Shown once: a stray Escape must not throw it away before it is copied.
    await user.keyboard('{Escape}');
    expect(screen.getByText('reset-token-shown-once')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Close modal' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes the rejection modal over the review drawer first, then the drawer', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    renderWithQuery(<CoachReviewDrawer coach={COACH} isOpen onClose={onClose} />);

    await user.click(screen.getByRole('button', { name: 'Reject' }));
    expect(screen.getByRole('dialog', { name: 'Reject Agent' })).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Reject Agent' })).not.toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Review Agent' })).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
