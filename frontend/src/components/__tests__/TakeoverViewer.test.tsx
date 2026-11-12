// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the login takeover viewer — exact text in (repeated letters, paste, IME), keys, pointer coordinates, statuses, its one socket
// ABOUTME: Red if a typed letter is dropped or doubled, a tap lands off its frame pixel, the socket redials, or a close is not sent

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, waitFor, fireEvent } from '@testing-library/react';
import { i18n } from '@pierre/i18n';
import TakeoverViewer, { type TakeoverViewerProps } from '../takeover/TakeoverViewer';
import { FakeTakeoverSocket, drawImage, installFakeTakeoverSocket, lastSocket } from '../../test/fakeTakeoverSocket';

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteTakeoverSocketUrl: (token: string) =>
      `ws://app.dravr.test/ws/providers/sciotte/takeover?token=${encodeURIComponent(token)}`,
  },
}));

const TOKEN = 'header.payload.signature';

function viewer(overrides: Partial<TakeoverViewerProps> = {}) {
  const props: TakeoverViewerProps = {
    token: TOKEN,
    providerLabel: 'Garmin',
    onOutcome: vi.fn(),
    onEnded: vi.fn(),
    ...overrides,
  };
  return { props, ...render(<TakeoverViewer {...props} />) };
}

/** Render, let the socket dial, and have the server say the page streams. */
async function streaming(overrides: Partial<TakeoverViewerProps> = {}) {
  const rendered = viewer(overrides);
  await waitFor(() => expect(FakeTakeoverSocket.instances).toHaveLength(1));
  const socket = lastSocket();
  act(() => socket.ready());
  await screen.findByText(i18n.t('shell.sciotteTakeoverPrompt', { provider: 'Garmin' }));
  return { ...rendered, socket, field: screen.getByTestId('takeover-keyboard-field') as HTMLInputElement };
}

/** A beforeinput as a browser dispatches it on the hidden field. */
function beforeInput(field: HTMLInputElement, inputType: string, data: string | null = null): InputEvent {
  const event = new InputEvent('beforeinput', { inputType, data, bubbles: true, cancelable: true });
  act(() => {
    field.dispatchEvent(event);
  });
  return event;
}

describe('TakeoverViewer', () => {
  beforeEach(() => {
    installFakeTakeoverSocket();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('dials the takeover socket with its token, and says it is opening the provider', async () => {
    viewer();
    expect(screen.getByTestId('takeover-status')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverConnecting', { provider: 'Garmin' }),
    );
    await waitFor(() => expect(FakeTakeoverSocket.instances).toHaveLength(1));
    expect(lastSocket().url).toBe(`ws://app.dravr.test/ws/providers/sciotte/takeover?token=${encodeURIComponent(TOKEN)}`);
  });

  it('shows the prompt once the page streams, and names the view for assistive tech', async () => {
    await streaming();
    expect(screen.getByRole('img', { name: i18n.t('shell.sciotteTakeoverViewAria', { provider: 'Garmin' }) })).toBeInTheDocument();
  });

  it('draws each frame at its own pixel size', async () => {
    const { socket } = await streaming();
    act(() => socket.frame(390, 844));
    const canvas = screen.getByTestId('takeover-canvas') as HTMLCanvasElement;
    await waitFor(() => expect(drawImage).toHaveBeenCalledTimes(1));
    expect(canvas.width).toBe(390);
    expect(canvas.height).toBe(844);
  });

  it('sends typed text exactly, repeated letters included, and keeps the field empty', async () => {
    const { socket, field } = await streaming();
    const events = ['p', 'a', 's', 's', 'w', 'o', 'o', 'r', 'd'].map((c) => beforeInput(field, 'insertText', c));
    expect(events.every((e) => e.defaultPrevented)).toBe(true);
    expect(socket.inputs()).toEqual(
      ['p', 'a', 's', 's', 'w', 'o', 'o', 'r', 'd'].map((text) => ({ type: 'text', text })),
    );
    expect(field.value).toBe('');
  });

  it("sends a soft keyboard's delete and return as key presses", async () => {
    const { socket, field } = await streaming();
    beforeInput(field, 'deleteContentBackward');
    beforeInput(field, 'insertLineBreak');
    expect(socket.inputs()).toEqual([
      { type: 'key', action: 'down', key: 'Backspace' },
      { type: 'key', action: 'up', key: 'Backspace' },
      { type: 'key', action: 'down', key: 'Enter' },
      { type: 'key', action: 'up', key: 'Enter' },
    ]);
  });

  it('sends a paste as its text, in pieces the server takes', async () => {
    const { socket, field } = await streaming();
    const paste = new Event('paste', { bubbles: true, cancelable: true });
    const long = 'k'.repeat(1500);
    Object.defineProperty(paste, 'clipboardData', { value: { getData: () => long } });
    act(() => {
      field.dispatchEvent(paste);
    });
    expect(paste.defaultPrevented).toBe(true);
    expect(socket.inputs()).toEqual([
      { type: 'text', text: 'k'.repeat(1024) },
      { type: 'text', text: 'k'.repeat(476) },
    ]);
  });

  it('forwards composed text once, when the composition ends', async () => {
    const { socket, field } = await streaming();
    // Composition is the IME's: its beforeinput is left alone.
    const composing = new InputEvent('beforeinput', {
      inputType: 'insertCompositionText',
      data: 'é',
      isComposing: true,
      cancelable: true,
    });
    act(() => {
      field.dispatchEvent(composing);
    });
    expect(composing.defaultPrevented).toBe(false);
    field.value = 'éte';
    act(() => {
      field.dispatchEvent(new Event('compositionend'));
    });
    expect(socket.inputs()).toEqual([{ type: 'text', text: 'éte' }]);
    expect(field.value).toBe('');
  });

  it("forwards the page's keys down and up, keeps them from the dialog, and leaves Shift+Tab as the way out", async () => {
    const { socket, field } = await streaming();
    const documentKeys = vi.fn();
    document.addEventListener('keydown', documentKeys);
    try {
      fireEvent.keyDown(field, { key: 'Enter' });
      fireEvent.keyUp(field, { key: 'Enter' });
      fireEvent.keyDown(field, { key: 'Escape' });
      fireEvent.keyDown(field, { key: 'Tab' });
      expect(documentKeys).not.toHaveBeenCalled();

      fireEvent.keyDown(field, { key: 'Tab', shiftKey: true });
      fireEvent.keyDown(field, { key: 'a' });
      fireEvent.keyDown(field, { key: 'v', metaKey: true });
      expect(documentKeys).toHaveBeenCalledTimes(3);
    } finally {
      document.removeEventListener('keydown', documentKeys);
    }
    expect(socket.inputs()).toEqual([
      { type: 'key', action: 'down', key: 'Enter' },
      { type: 'key', action: 'up', key: 'Enter' },
      { type: 'key', action: 'down', key: 'Escape' },
      { type: 'key', action: 'down', key: 'Tab' },
    ]);
  });

  it("maps a pointer on the drawn canvas to the frame's own pixels", async () => {
    const { socket } = await streaming();
    act(() => socket.frame(400, 800));
    await waitFor(() => expect(drawImage).toHaveBeenCalled());
    const canvas = screen.getByTestId('takeover-canvas');
    // Drawn at half size, 10px in and 20px down.
    canvas.getBoundingClientRect = () => ({ left: 10, top: 20, width: 200, height: 400 }) as DOMRect;

    fireEvent.pointerDown(canvas, { clientX: 110, clientY: 120, pointerType: 'touch', pointerId: 1 });
    fireEvent.pointerUp(canvas, { clientX: 60, clientY: 420, pointerType: 'touch', pointerId: 1 });

    expect(socket.inputs()).toEqual([
      { type: 'pointer', action: 'down', x: 200, y: 200 },
      { type: 'pointer', action: 'up', x: 100, y: 800 },
    ]);
  });

  it("sizes the page's viewport to the view once it streams, in whole pixels", async () => {
    const width = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth');
    const height = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight');
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 359.6 });
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => 640 });
    try {
      const { socket } = await streaming();
      expect(socket.inputs()).toEqual([{ type: 'resize', width: 360, height: 640 }]);
    } finally {
      if (width) Object.defineProperty(HTMLElement.prototype, 'clientWidth', width);
      if (height) Object.defineProperty(HTMLElement.prototype, 'clientHeight', height);
    }
  });

  it('focuses the keyboard field from the keyboard button, for a touch screen', async () => {
    const { field } = await streaming();
    fireEvent.click(screen.getByRole('button', { name: i18n.t('shell.sciotteTakeoverKeyboard') }));
    expect(document.activeElement).toBe(field);
  });

  it('says why a popup did nothing, and why the page came back', async () => {
    const { socket } = await streaming();
    act(() => socket.event({ type: 'popup_refused' }));
    expect(screen.getByTestId('takeover-notice')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverPopupRefused', { provider: 'Garmin' }),
    );
    act(() => socket.event({ type: 'navigation_reset' }));
    expect(screen.getByTestId('takeover-notice')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverPageReset', { provider: 'Garmin' }),
    );
  });

  it('offers the other way to connect when the window has one', async () => {
    await streaming({ oauthOffered: true });
    expect(screen.getByTestId('takeover-oauth-offer')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverOAuthOffer', { provider: 'Garmin' }),
    );
  });

  it('hands the outcome to its window, says it is finishing, and does not read the close after it as an end', async () => {
    const { socket, props } = await streaming();
    const outcome = { type: 'outcome', status_code: 200, body: { status: 'connected', provider: 'sciotte_garmin' } };
    act(() => socket.event(outcome));
    expect(props.onOutcome).toHaveBeenCalledWith(outcome, {
      kind: 'answer',
      data: { status: 'connected', provider: 'sciotte_garmin' },
    });
    expect(screen.getByTestId('takeover-status')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverFinishing', { provider: 'Garmin' }),
    );
    act(() => socket.serverClose());
    expect(props.onEnded).not.toHaveBeenCalled();
  });

  it('says the sign-in window closed when the socket ends with no outcome', async () => {
    const { socket, props } = await streaming();
    act(() => socket.serverClose());
    expect(props.onEnded).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('takeover-status')).toHaveTextContent(
      i18n.t('shell.sciotteTakeoverEnded', { provider: 'Garmin' }),
    );
  });

  it('sends nothing before the page streams', async () => {
    viewer();
    await waitFor(() => expect(FakeTakeoverSocket.instances).toHaveLength(1));
    const socket = lastSocket();
    socket.open();
    beforeInput(screen.getByTestId('takeover-keyboard-field') as HTMLInputElement, 'insertText', 'x');
    expect(socket.sent).toEqual([]);
  });

  it('keeps its one socket across re-renders, and closes it when the window goes', async () => {
    const { socket, rerender, unmount, props } = await streaming();
    rerender(<TakeoverViewer {...props} onOutcome={vi.fn()} onEnded={vi.fn()} providerLabel="Garmin" />);
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(FakeTakeoverSocket.instances).toHaveLength(1);
    expect(socket.closes).toEqual([]);

    unmount();
    expect(socket.closes).toEqual([1000]);
  });

  it('never opens a socket for a mount torn down at once', async () => {
    const { unmount } = viewer();
    unmount();
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(FakeTakeoverSocket.instances).toHaveLength(0);
  });
});
