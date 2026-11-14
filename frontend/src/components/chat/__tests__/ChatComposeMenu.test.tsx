// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the chat "+" menu as a layer: one Escape closes the menu, never the sheet it opens in
// ABOUTME: The group-name dialog it opens is a layer of its own over that sheet

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ChatComposeMenu from '../ChatComposeMenu';
import { Sheet } from '../../ui/Sheet';

function renderInSheet() {
  const onSheetClose = vi.fn();
  render(
    <Sheet side="left" title="History" onClose={onSheetClose}>
      <ChatComposeMenu onNewChat={vi.fn()} onNewGroupChat={vi.fn()} />
    </Sheet>,
  );
  return onSheetClose;
}

describe('ChatComposeMenu', () => {
  it('closes on Escape and leaves the sheet it opened in up', async () => {
    const user = userEvent.setup();
    const onSheetClose = renderInSheet();

    await user.click(screen.getByRole('button', { name: 'New' }));
    expect(screen.getByTestId('chat-compose-menu')).toBeInTheDocument();

    await user.keyboard('{Escape}');
    // carnet#1553: the menu and the sheet both listened on `document`.
    expect(screen.queryByTestId('chat-compose-menu')).not.toBeInTheDocument();
    expect(onSheetClose).not.toHaveBeenCalled();

    await user.keyboard('{Escape}');
    expect(onSheetClose).toHaveBeenCalledTimes(1);
  });

  it('closes the group-name dialog on Escape and leaves the sheet up', async () => {
    const user = userEvent.setup();
    const onSheetClose = renderInSheet();

    await user.click(screen.getByRole('button', { name: 'New' }));
    await user.click(screen.getByRole('menuitem', { name: 'New group chat' }));
    expect(screen.getByTestId('group-name-input')).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(screen.queryByTestId('group-name-input')).not.toBeInTheDocument();
    expect(onSheetClose).not.toHaveBeenCalled();
  });
});
