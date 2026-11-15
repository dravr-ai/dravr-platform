// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the phone drawer on useDialog — Tab stays inside, the page behind freezes, closing hands focus back
// ABOUTME: And it is one layer among the others: Escape closes a dialog over it first, then the drawer

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MobileDrawer } from '../MobileNav';
import { Modal } from '../../ui/Modal';

function Drawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <MobileDrawer
      open={open}
      onClose={onClose}
      secondary={[{ id: 'groups', name: 'Groups', icon: <span /> }]}
      activeTab="home"
      onSelect={vi.fn()}
      userLabel="Alex Rider"
      userInitial="A"
      onOpenSettings={vi.fn()}
      onSendFeedback={vi.fn()}
      onSignOut={vi.fn()}
      feedbackUnread={0}
    />
  );
}

describe('MobileDrawer', () => {
  it('moves focus in on open and wraps Tab and Shift+Tab inside the panel', async () => {
    const user = userEvent.setup();
    render(<Drawer open onClose={vi.fn()} />);
    const close = screen.getByRole('button', { name: 'Close menu' });
    const signOut = screen.getByRole('button', { name: 'Sign out' });

    expect(close).toHaveFocus();

    // Its own trap filtered on `offsetParent`, which is null inside the
    // drawer's `position: fixed` root — the list came back empty and Tab
    // walked out to the tab bar behind.
    signOut.focus();
    await user.tab();
    expect(close).toHaveFocus();

    await user.tab({ shift: true });
    expect(signOut).toHaveFocus();
  });

  it('is inert while closed and takes focus once opened', () => {
    const view = render(<Drawer open={false} onClose={vi.fn()} />);
    const root = document.querySelector('[aria-hidden="true"].fixed');
    if (!(root instanceof HTMLElement)) throw new Error('no drawer root');
    expect(root).toHaveAttribute('inert');

    view.rerender(<Drawer open onClose={vi.fn()} />);
    // The attribute is gone before useDialog moves focus in: an inert
    // panel would refuse it.
    expect(root).not.toHaveAttribute('inert');
    expect(screen.getByRole('button', { name: 'Close menu' })).toHaveFocus();
  });

  it('freezes the page while open and hands focus back to its opener on close', () => {
    const opener = document.createElement('button');
    document.body.appendChild(opener);
    opener.focus();

    const view = render(<Drawer open onClose={vi.fn()} />);
    expect(document.body.style.overflow).toBe('hidden');

    view.rerender(<Drawer open={false} onClose={vi.fn()} />);
    expect(document.body.style.overflow).toBe('');
    expect(opener).toHaveFocus();
    opener.remove();
  });

  it('keeps the page frozen while a dialog over it is still up', () => {
    const view = render(
      <>
        <Drawer open onClose={vi.fn()} />
        <Modal isOpen onClose={vi.fn()} title="Sign out?">
          <p>sure</p>
        </Modal>
      </>,
    );

    // It saved and restored `body.style.overflow` on its own, so closing the
    // drawer under a dialog thawed the page while the dialog was up.
    view.rerender(
      <>
        <Drawer open={false} onClose={vi.fn()} />
        <Modal isOpen onClose={vi.fn()} title="Sign out?">
          <p>sure</p>
        </Modal>
      </>,
    );
    expect(document.body.style.overflow).toBe('hidden');
  });

  it('lets one Escape close the dialog over it, and the next close the drawer', async () => {
    const user = userEvent.setup();
    const onDrawerClose = vi.fn();
    const onModalClose = vi.fn();
    render(
      <>
        <Drawer open onClose={onDrawerClose} />
        <Modal isOpen onClose={onModalClose} title="Sign out?">
          <p>sure</p>
        </Modal>
      </>,
    );

    await user.keyboard('{Escape}');
    expect(onModalClose).toHaveBeenCalledTimes(1);
    expect(onDrawerClose).not.toHaveBeenCalled();
  });

  it('closes on Escape when it is the top layer', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Drawer open onClose={onClose} />);

    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
