// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the confirm dialog's name — its own heading names it and its message describes it
// ABOUTME: It drew a heading but passed Modal no title, so a screen reader announced an unnamed dialog

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
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
});
