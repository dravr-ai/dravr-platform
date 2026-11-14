// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves Tab cannot leave an open dialog and that closing returns focus to the opener
// ABOUTME: With dialogs and popovers stacked, Escape reaches only the top layer and Tab the top dialog, whatever order they leave in

import { StrictMode, useState, type ReactNode } from 'react';
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useDialog, useEscapeLayer } from '../useDialog';

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

/** A dialog named by `name`, its three controls labelled after it. */
function Named({
  name,
  open,
  onClose,
  closeOnEscape,
  children,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
  closeOnEscape?: boolean;
  children?: ReactNode;
}) {
  const { containerRef } = useDialog({ open, onClose, closeOnEscape });
  if (!open) return null;
  return (
    <div ref={containerRef} tabIndex={-1} role="dialog" aria-modal="true" aria-label={name}>
      <button type="button">{`${name} first`}</button>
      <a href={`#${name}`}>{`${name} middle`}</a>
      {children}
      <button type="button">{`${name} last`}</button>
    </div>
  );
}

/** A menu that is a layer over whatever it opens in, and nothing more. */
function Popover({ open, onEscape }: { open: boolean; onEscape: () => void }) {
  const ref = useEscapeLayer(open, onEscape);
  return open ? <div ref={ref} role="menu" aria-label="popover" /> : null;
}

/** Two dialogs that each close themselves, the upper opened after the lower. */
function Stacked() {
  const [lower, setLower] = useState(true);
  const [upper, setUpper] = useState(true);
  return (
    <>
      <Named name="lower" open={lower} onClose={() => setLower(false)} />
      <Named name="upper" open={upper} onClose={() => setUpper(false)} />
    </>
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

  describe('stacked dialogs', () => {
    it('closes only the dialog on top on one Escape, and the one under it on the next', async () => {
      const user = userEvent.setup();
      render(<Stacked />);

      await user.keyboard('{Escape}');
      // Both used to close: each dialog listened on `document` for itself.
      expect(screen.queryByRole('dialog', { name: 'upper' })).not.toBeInTheDocument();
      expect(screen.getByRole('dialog', { name: 'lower' })).toBeInTheDocument();
      expect(document.body.style.overflow).toBe('hidden');

      await user.keyboard('{Escape}');
      expect(screen.queryByRole('dialog', { name: 'lower' })).not.toBeInTheDocument();
      expect(document.body.style.overflow).toBe('');
    });

    it('keeps Escape on the top dialog when the one under it closes first', () => {
      const lowerClose = vi.fn();
      const upperClose = vi.fn();
      const view = render(
        <>
          <Named name="lower" open onClose={lowerClose} />
          <Named name="upper" open onClose={upperClose} />
        </>,
      );

      view.rerender(
        <>
          <Named name="lower" open={false} onClose={lowerClose} />
          <Named name="upper" open onClose={upperClose} />
        </>,
      );
      expect(document.body.style.overflow).toBe('hidden');

      fireEvent.keyDown(document, { key: 'Escape' });
      expect(upperClose).toHaveBeenCalledTimes(1);
      expect(lowerClose).not.toHaveBeenCalled();
    });

    it('hands Escape down when the top dialog unmounts without closing', () => {
      const lowerClose = vi.fn();
      const upperClose = vi.fn();
      const view = render(
        <>
          <Named name="lower" open onClose={lowerClose} />
          <Named name="upper" open onClose={upperClose} />
        </>,
      );

      // Its parent went away with it still `open` — no false edge ever ran.
      view.rerender(<Named name="lower" open onClose={lowerClose} />);

      fireEvent.keyDown(document, { key: 'Escape' });
      expect(lowerClose).toHaveBeenCalledTimes(1);
      expect(upperClose).not.toHaveBeenCalled();
    });

    it('leaves no stale entry behind under StrictMode', async () => {
      const user = userEvent.setup();
      // StrictMode mounts, cleans up and mounts every effect again: a stack
      // that popped instead of removing by identity, or pushed without
      // removing, would answer the second Escape with the wrong dialog.
      render(
        <StrictMode>
          <Stacked />
        </StrictMode>,
      );

      await user.keyboard('{Escape}');
      expect(screen.queryByRole('dialog', { name: 'upper' })).not.toBeInTheDocument();
      expect(screen.getByRole('dialog', { name: 'lower' })).toBeInTheDocument();

      await user.keyboard('{Escape}');
      expect(screen.queryByRole('dialog', { name: 'lower' })).not.toBeInTheDocument();
      expect(document.body.style.overflow).toBe('');
    });

    it('returns focus to the opener under StrictMode too', () => {
      const opener = document.createElement('button');
      document.body.appendChild(opener);
      opener.focus();

      const view = render(
        <StrictMode>
          <Harness open onClose={() => {}} />
        </StrictMode>,
      );
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'first' }));

      view.rerender(
        <StrictMode>
          <Harness open={false} onClose={() => {}} />
        </StrictMode>,
      );
      // StrictMode's rehearsal cleanup ran with the ref already detached, so
      // it saw focus "outside" the dialog, kept it there, and the rerun took
      // the dialog's own first control as the opener to return to.
      expect(document.activeElement).toBe(opener);
      opener.remove();
    });

    it('keeps a dialog inside another on top when both open in the same render', () => {
      const outerClose = vi.fn();
      const innerClose = vi.fn();
      render(
        <Named name="outer" open onClose={outerClose}>
          <Named name="inner" open onClose={innerClose} />
        </Named>,
      );

      fireEvent.keyDown(document, { key: 'Escape' });
      expect(innerClose).toHaveBeenCalledTimes(1);
      expect(outerClose).not.toHaveBeenCalled();
    });

    it('lets a top dialog that must be resolved shield the ones under it', () => {
      const lowerClose = vi.fn();
      const upperClose = vi.fn();
      render(
        <>
          <Named name="lower" open onClose={lowerClose} />
          <Named name="upper" open onClose={upperClose} closeOnEscape={false} />
        </>,
      );

      fireEvent.keyDown(document, { key: 'Escape' });
      expect(upperClose).not.toHaveBeenCalled();
      expect(lowerClose).not.toHaveBeenCalled();
    });

    it('lets Tab move within the top dialog, untouched by the trap of the one under it', async () => {
      const user = userEvent.setup();
      render(<Stacked />);

      screen.getByRole('link', { name: 'upper middle' }).focus();
      await user.tab();

      // The lower dialog's trap saw focus outside its own container and
      // pulled it to "lower first"; the upper's then pulled it on to "upper
      // first", so Tab from the middle never reached "upper last".
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'upper last' }));
    });
  });

  describe('popover layers', () => {
    /** A dialog with a menu inside it, the menu opened after the dialog. */
    function DialogWithMenu() {
      const [dialog, setDialog] = useState(true);
      const [menu, setMenu] = useState(false);
      return (
        <Named name="sheet" open={dialog} onClose={() => setDialog(false)}>
          <button type="button" onClick={() => setMenu(true)}>
            open menu
          </button>
          <Popover open={menu} onEscape={() => setMenu(false)} />
        </Named>
      );
    }

    it('closes the menu on one Escape and the dialog under it on the next', async () => {
      const user = userEvent.setup();
      render(<DialogWithMenu />);

      await user.click(screen.getByRole('button', { name: 'open menu' }));
      expect(screen.getByRole('menu', { name: 'popover' })).toBeInTheDocument();

      await user.keyboard('{Escape}');
      expect(screen.queryByRole('menu', { name: 'popover' })).not.toBeInTheDocument();
      expect(screen.getByRole('dialog', { name: 'sheet' })).toBeInTheDocument();

      await user.keyboard('{Escape}');
      expect(screen.queryByRole('dialog', { name: 'sheet' })).not.toBeInTheDocument();
    });

    it('leaves Tab to the dialog under an open menu', async () => {
      const user = userEvent.setup();
      render(<DialogWithMenu />);
      await user.click(screen.getByRole('button', { name: 'open menu' }));

      screen.getByRole('button', { name: 'sheet last' }).focus();
      await user.tab();

      // The menu takes Escape only: the sheet's trap still wraps Tab.
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'sheet first' }));
    });

    it('freezes nothing behind it', () => {
      const view = render(<Popover open onEscape={() => {}} />);
      expect(document.body.style.overflow).toBe('');

      // Nor does it hold the lock open after the dialog under it closes.
      view.rerender(
        <>
          <Named name="sheet" open onClose={() => {}} />
          <Popover open onEscape={() => {}} />
        </>,
      );
      expect(document.body.style.overflow).toBe('hidden');
      view.rerender(
        <>
          <Named name="sheet" open={false} onClose={() => {}} />
          <Popover open onEscape={() => {}} />
        </>,
      );
      expect(document.body.style.overflow).toBe('');
    });

    it('stays above the dialog it sits in when both open in the same render', () => {
      // React runs the menu's effect before the dialog's, so the dialog
      // registers last; by push order alone it would take the first Escape.
      const onEscape = vi.fn();
      const onClose = vi.fn();
      render(
        <Named name="sheet" open onClose={onClose}>
          <Popover open onEscape={onEscape} />
        </Named>,
      );

      fireEvent.keyDown(document, { key: 'Escape' });
      expect(onEscape).toHaveBeenCalledTimes(1);
      expect(onClose).not.toHaveBeenCalled();
    });

    it('closes nothing on an Escape a control inside already handled', () => {
      const onEscape = vi.fn();
      const onClose = vi.fn();
      render(
        <>
          <Named name="sheet" open onClose={onClose} />
          <Popover open onEscape={onEscape} />
        </>,
      );

      // The history sheet's rename field cancels the rename on Escape and
      // marks the key handled; the sheet stays up.
      const handled = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
      handled.preventDefault();
      document.dispatchEvent(handled);
      expect(onEscape).not.toHaveBeenCalled();
      expect(onClose).not.toHaveBeenCalled();

      fireEvent.keyDown(document, { key: 'Escape' });
      expect(onEscape).toHaveBeenCalledTimes(1);
      expect(onClose).not.toHaveBeenCalled();
    });
  });
});
