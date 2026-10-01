// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins when a waiting build is taken silently and when the reload strip asks first
// ABOUTME: Silent with no typed text; asks over a signed-in draft, in whichever tab took the build; waits on the sign-in form

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, fireEvent, render, screen, cleanup } from '@testing-library/react';
import ServiceWorkerUpdatePrompt from '../ServiceWorkerUpdatePrompt';
import { typedTextWouldBeLost } from '../../utils/typedText';

const sw = vi.hoisted(() => ({
  needRefresh: false,
  setNeedRefresh: vi.fn(),
  updateServiceWorker: vi.fn(() => Promise.resolve()),
  // What the plugin calls once the new worker controls the page, in every tab
  // that was told a build is waiting — not only the tab that took it.
  onNeedReload: undefined as (() => void) | undefined,
}));
const auth = vi.hoisted(() => ({ isAuthenticated: true }));

vi.mock('virtual:pwa-register/react', () => ({
  useRegisterSW: (options: { onNeedReload?: () => void }) => {
    sw.onNeedReload = options.onNeedReload;
    return {
      needRefresh: [sw.needRefresh, sw.setNeedRefresh],
      updateServiceWorker: sw.updateServiceWorker,
    };
  },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ isAuthenticated: auth.isAuthenticated }),
}));

/** Put a field on the page as if the athlete had typed `value` into it. */
function typed(tag: 'textarea' | 'input', value: string, type?: string): HTMLElement {
  const field = document.createElement(tag);
  if (type) {
    field.setAttribute('type', type);
  }
  field.value = value;
  document.body.appendChild(field);
  return field;
}

const reload = vi.fn();

describe('ServiceWorkerUpdatePrompt', () => {
  beforeEach(() => {
    sw.needRefresh = false;
    auth.isAuthenticated = true;
    sw.updateServiceWorker.mockClear();
    sw.setNeedRefresh.mockClear();
    sw.onNeedReload = undefined;
    reload.mockClear();
    // jsdom's own location.reload is not implemented and cannot be spied on.
    vi.stubGlobal('location', { ...window.location, reload });
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
  });

  it('does nothing while no build is waiting', () => {
    render(<ServiceWorkerUpdatePrompt />);
    expect(screen.queryByTestId('sw-update-prompt')).toBeNull();
    expect(sw.updateServiceWorker).not.toHaveBeenCalled();
  });

  it('takes a waiting build without asking when nothing is typed', () => {
    sw.needRefresh = true;
    render(<ServiceWorkerUpdatePrompt />);
    expect(sw.updateServiceWorker).toHaveBeenCalledTimes(1);
    expect(sw.updateServiceWorker).toHaveBeenCalledWith(true);
    expect(screen.queryByTestId('sw-update-prompt')).toBeNull();
  });

  it('asks, and does not reload, over a signed-in draft', () => {
    sw.needRefresh = true;
    typed('textarea', 'Ma sortie de dimanche était');
    render(<ServiceWorkerUpdatePrompt />);
    expect(screen.getByTestId('sw-update-prompt')).toBeTruthy();
    expect(sw.updateServiceWorker).not.toHaveBeenCalled();
  });

  it('neither asks nor reloads over credentials on the sign-in form', () => {
    sw.needRefresh = true;
    auth.isAuthenticated = false;
    typed('input', 'alice@acme.com', 'email');
    render(<ServiceWorkerUpdatePrompt />);
    expect(screen.queryByTestId('sw-update-prompt')).toBeNull();
    expect(sw.updateServiceWorker).not.toHaveBeenCalled();
  });

  it('takes the build once the session opens and the sign-in form is gone', () => {
    sw.needRefresh = true;
    auth.isAuthenticated = false;
    const email = typed('input', 'alice@acme.com', 'email');
    const { rerender } = render(<ServiceWorkerUpdatePrompt />);
    expect(sw.updateServiceWorker).not.toHaveBeenCalled();

    email.remove();
    auth.isAuthenticated = true;
    rerender(<ServiceWorkerUpdatePrompt />);
    expect(sw.updateServiceWorker).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('sw-update-prompt')).toBeNull();
  });

  it('keeps the old build when the athlete answers later', () => {
    sw.needRefresh = true;
    typed('textarea', 'Ma sortie de dimanche était');
    render(<ServiceWorkerUpdatePrompt />);
    const [later] = screen.getAllByRole('button');
    fireEvent.click(later);
    expect(sw.setNeedRefresh).toHaveBeenCalledWith(false);
    expect(sw.updateServiceWorker).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
  });

  it('reloads over the draft once the athlete asks for the build', () => {
    sw.needRefresh = true;
    typed('textarea', 'Ma sortie de dimanche était');
    render(<ServiceWorkerUpdatePrompt />);
    const [, takeBuild] = screen.getAllByRole('button');
    fireEvent.click(takeBuild);
    expect(sw.updateServiceWorker).toHaveBeenCalledTimes(1);
    expect(sw.updateServiceWorker).toHaveBeenCalledWith(true);

    act(() => sw.onNeedReload?.());
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('reloads when another tab takes the build and nothing is typed here', () => {
    render(<ServiceWorkerUpdatePrompt />);
    act(() => sw.onNeedReload?.());
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('does not reload a draft away when another tab takes the build', () => {
    sw.needRefresh = true;
    typed('textarea', 'Ma sortie de dimanche était');
    render(<ServiceWorkerUpdatePrompt />);

    act(() => sw.onNeedReload?.());
    expect(reload).not.toHaveBeenCalled();
    expect(screen.getByTestId('sw-update-prompt')).toBeTruthy();

    // No worker is waiting any more, so the button reloads the page itself.
    const [, takeBuild] = screen.getAllByRole('button');
    fireEvent.click(takeBuild);
    expect(reload).toHaveBeenCalledTimes(1);
    expect(sw.updateServiceWorker).not.toHaveBeenCalled();
  });
});

describe('typedTextWouldBeLost', () => {
  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('is false on a page with no typed text', () => {
    typed('textarea', '   ');
    typed('input', 'on', 'checkbox');
    expect(typedTextWouldBeLost()).toBe(false);
  });

  it('is true for a non-empty text field, textarea or editable region', () => {
    typed('input', 'berlin', 'search');
    expect(typedTextWouldBeLost()).toBe(true);
    document.body.innerHTML = '';

    typed('textarea', 'draft');
    expect(typedTextWouldBeLost()).toBe(true);
    document.body.innerHTML = '';

    const region = document.createElement('div');
    region.setAttribute('contenteditable', 'true');
    region.textContent = 'draft';
    document.body.appendChild(region);
    expect(typedTextWouldBeLost()).toBe(true);
  });
});
