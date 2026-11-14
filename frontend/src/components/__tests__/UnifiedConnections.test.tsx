// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The token screen's API Tokens tab follows `role`, the same field that decides the admin rail
// ABOUTME: `is_admin` has skewed from `role`, and an operator on it would land on a screen missing its token list

import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import UnifiedConnections from '../UnifiedConnections';

const viewer = vi.hoisted(() => ({ role: 'admin', is_admin: false }));

vi.mock('../../hooks/useAuth', () => ({ useAuth: () => ({ user: viewer }) }));
vi.mock('../ApiKeyList', () => ({ default: () => <div data-testid="token-list" /> }));
vi.mock('../A2AClientList', () => ({ default: () => <div data-testid="a2a-list" /> }));
// The form mints the token; the stand-in hands back a fixed one.
vi.mock('../CreateApiKey', () => ({
  default: ({ onTokenCreated }: { onTokenCreated: (response: unknown) => void }) => (
    <button
      type="button"
      onClick={() =>
        onTokenCreated({
          token_id: 'token-9',
          service_name: 'ci',
          jwt_token: 'pierre_at_minted_jwt',
          token_prefix: 'pierre_at_min',
          is_super_admin: false,
          expires_at: null,
        })
      }
    >
      mint
    </button>
  ),
}));

describe('UnifiedConnections', () => {
  it('opens on the token list for a role admin whose is_admin is false', () => {
    viewer.role = 'admin';
    viewer.is_admin = false;
    render(<UnifiedConnections />);
    expect(screen.getByTestId('token-list')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'API Tokens' })).toBeInTheDocument();
  });

  it('offers a plain user only the OAuth apps, whatever a stale is_admin says', () => {
    viewer.role = 'user';
    viewer.is_admin = true;
    render(<UnifiedConnections />);
    expect(screen.getByTestId('a2a-list')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'API Tokens' })).toBeNull();
  });

  it('shows a minted token as a dialog that holds focus and outlasts an Escape', () => {
    viewer.role = 'admin';
    viewer.is_admin = false;
    render(<UnifiedConnections />);
    fireEvent.click(screen.getByRole('button', { name: /Create API Token/ }));
    fireEvent.click(screen.getByRole('button', { name: 'mint' }));

    const dialog = screen.getByRole('dialog', { name: 'API Token Generated Successfully' });
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    // Shown once: a stray Escape must not throw it away before it is copied.
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.getByRole('dialog', { name: 'API Token Generated Successfully' })).toBeInTheDocument();
  });
});
