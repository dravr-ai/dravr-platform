// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Sheet — a dialog named by its title that Escape, the close button and the scrim all close
// ABOUTME: Its extra header actions sit beside the close button; one Escape closes only the sheet on top of a stack

import { useState } from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sheet } from '../Sheet';

function renderSheet(onClose = vi.fn()) {
  render(
    <Sheet side="bottom" title="Today" onClose={onClose} actions={<button type="button">New</button>}>
      <p>today’s session</p>
    </Sheet>,
  );
  return onClose;
}

/** Home's month sheet with a plan day's evidence sheet opened from inside it. */
function MonthWithEvidence() {
  const [month, setMonth] = useState(true);
  const [evidence, setEvidence] = useState(false);
  return (
    month && (
      <Sheet side="bottom" title="Month" onClose={() => setMonth(false)}>
        <button type="button" onClick={() => setEvidence(true)}>
          Evidence
        </button>
        {/* Rendered from inside the month sheet, as EvidenceChip is. */}
        {evidence && (
          <Sheet side="right" title="Evidence" onClose={() => setEvidence(false)}>
            <p>the studies behind it</p>
          </Sheet>
        )}
      </Sheet>
    )
  );
}

describe('Sheet', () => {
  it('is a modal dialog named by its title, with its actions and content', () => {
    renderSheet();
    const dialog = screen.getByRole('dialog', { name: 'Today' });
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(screen.getByRole('button', { name: 'New' })).toBeInTheDocument();
    expect(screen.getByText('today’s session')).toBeInTheDocument();
  });

  it('closes on Escape', () => {
    const onClose = renderSheet();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes from its close button and from the scrim', async () => {
    const user = userEvent.setup();
    const onClose = renderSheet();
    await user.click(screen.getByRole('button', { name: 'Close' }));
    expect(onClose).toHaveBeenCalledTimes(1);

    const scrim = document.querySelector('.bg-scrim\\/60');
    if (!scrim) throw new Error('no scrim');
    await user.click(scrim);
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('closes only the sheet on top when one is opened over another', async () => {
    const user = userEvent.setup();
    render(<MonthWithEvidence />);

    await user.click(screen.getByRole('button', { name: 'Evidence' }));
    expect(screen.getByRole('dialog', { name: 'Evidence' })).toBeInTheDocument();

    await user.keyboard('{Escape}');
    // carnet#1553: the month sheet closed with the evidence sheet over it.
    expect(screen.queryByRole('dialog', { name: 'Evidence' })).not.toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Month' })).toBeInTheDocument();
    // Focus goes back to the control in the month sheet that opened it.
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Evidence' }));

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Month' })).not.toBeInTheDocument();
  });
});
