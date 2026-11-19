// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The token screen opens on its API Tokens tab and shows a minted token once, in a dialog
// ABOUTME: Only an admin reaches it (the Dashboard's ADMIN_ONLY_TABS guard), so it asks nothing about the viewer

import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import UnifiedConnections from '../UnifiedConnections';

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
  it('opens on the token list, with the connected apps one tab away', () => {
    render(<UnifiedConnections />);
    expect(screen.getByTestId('token-list')).toBeInTheDocument();
    expect(screen.queryByTestId('a2a-list')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Connected Apps' }));
    expect(screen.getByTestId('a2a-list')).toBeInTheDocument();
    expect(screen.queryByTestId('token-list')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'API Tokens' }));
    expect(screen.getByTestId('token-list')).toBeInTheDocument();
  });

  it('shows a minted token as a dialog that holds focus and outlasts an Escape', () => {
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
