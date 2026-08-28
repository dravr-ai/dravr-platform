// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves Tab cannot leave an open dialog and that closing returns focus to the opener
// ABOUTME: aria-modal tells assistive tech the page is inert; it does not constrain the Tab key

import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useDialog } from '../useDialog';

function Harness({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { containerRef } = useDialog({ open, onClose });
  return (
    <div>
      <button type="button">outside-before</button>
      {open && (
        <div ref={containerRef} tabIndex={-1} role="dialog" aria-modal="true" data-testid="dialog">
          <button type="button">first</button>
          {/* A link rather than a text field: `a[href]` is in useDialog's
              FOCUSABLE list and had no coverage, and a raw form control here
              would count against the design system ratchet even from a
              fixture — which greps source text, comments included. */}
          <a href="#middle">middle</a>
          <button type="button">last</button>
        </div>
      )}
      <button type="button">outside-after</button>
    </div>
  );
}

afterEach(() => {
  cleanup();
  document.body.style.overflow = '';
});

describe('useDialog', () => {
  it('moves focus into the dialog on open, onto the first real control', async () => {
    render(<Harness open onClose={() => {}} />);
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'first' }));
  });

  it('wraps Tab from the last control back to the first', async () => {
    const user = userEvent.setup();
    render(<Harness open onClose={() => {}} />);

    screen.getByRole('button', { name: 'last' }).focus();
    await user.tab();

    // Without the trap this landed on "outside-after" — a live control on a
    // page the dialog claims is inert.
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'first' }));
  });

  it('wraps Shift+Tab from the first control round to the last', async () => {
    const user = userEvent.setup();
    render(<Harness open onClose={() => {}} />);

    screen.getByRole('button', { name: 'first' }).focus();
    await user.tab({ shift: true });

    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'last' }));
  });

  it('pulls focus back when it has escaped the dialog entirely', async () => {
    const user = userEvent.setup();
    render(<Harness open onClose={() => {}} />);

    screen.getByRole('button', { name: 'outside-before' }).focus();
    await user.tab();

    expect(screen.getByTestId('dialog')).toContainElement(
      document.activeElement as HTMLElement,
    );
  });

  it('returns focus to whatever opened it', async () => {
    const opener = document.createElement('button');
    opener.textContent = 'opener';
    document.body.appendChild(opener);
    opener.focus();
    expect(document.activeElement).toBe(opener);

    const { rerender } = render(<Harness open onClose={() => {}} />);
    expect(document.activeElement).not.toBe(opener);

    rerender(<Harness open={false} onClose={() => {}} />);

    // Previously focus was dropped on <body>, so the next Tab restarted from
    // the top of the document instead of where the user had been.
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it('closes on Escape', async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<Harness open onClose={onClose} />);

    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('freezes the page behind it and thaws it only when the LAST dialog closes', () => {
    const a = render(<Harness open onClose={() => {}} />);
    expect(document.body.style.overflow).toBe('hidden');

    const b = render(<Harness open onClose={() => {}} />);
    expect(document.body.style.overflow).toBe('hidden');

    // Closing the inner dialog used to unfreeze the page while the outer one
    // was still up, and the background scrolled underneath it.
    b.unmount();
    expect(document.body.style.overflow).toBe('hidden');

    a.unmount();
    expect(document.body.style.overflow).toBe('');
  });

  it('does nothing at all while closed', () => {
    render(<Harness open={false} onClose={() => {}} />);
    expect(document.body.style.overflow).toBe('');
    expect(screen.queryByTestId('dialog')).not.toBeInTheDocument();
  });
});
