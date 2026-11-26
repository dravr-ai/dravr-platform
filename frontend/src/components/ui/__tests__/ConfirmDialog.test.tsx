// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the confirm dialog's name — its own heading names it and its message describes it
// ABOUTME: and that a loading dialog refuses Confirm, Cancel and Escape until its action settles

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ConfirmDialog } from '../ConfirmDialog';

describe('ConfirmDialog', () => {
  it('is a dialog named by its heading and described by its message', () => {
    render(
      <ConfirmDialog
        isOpen
        onClose={vi.fn()}
        onConfirm={vi.fn()}
        title="Delete this conversation?"
        message="Its messages go with it."
      />,
    );

    const dialog = screen.getByRole('dialog', { name: 'Delete this conversation?' });
    expect(dialog).toHaveAccessibleDescription('Its messages go with it.');
    // Named by the heading it draws, not by a second one in Modal's header.
    expect(screen.getAllByRole('heading', { name: 'Delete this conversation?' })).toHaveLength(1);
  });

  it('holds both buttons and Escape while the confirmed action runs', () => {
    const onClose = vi.fn();
    const onConfirm = vi.fn();
    render(
      <ConfirmDialog
        isOpen
        onClose={onClose}
        onConfirm={onConfirm}
        title="Disconnect Garmin?"
        message="Its data stops syncing."
        confirmLabel="Disconnect"
        isLoading
      />,
    );

    const confirm = screen.getByRole('button', { name: 'Disconnect' });
    const cancel = screen.getByRole('button', { name: 'Cancel' });
    expect(confirm).toBeDisabled();
    expect(cancel).toBeDisabled();

    fireEvent.click(confirm);
    fireEvent.click(cancel);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });
});
