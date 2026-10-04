// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins when a waiting build is taken silently and when the reload strip asks first
// ABOUTME: Silent with no typed text or only pre-filled values; asks over an edited signed-in draft; waits on the sign-in form

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useState } from 'react';
import { act, fireEvent, render, screen, cleanup } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ServiceWorkerUpdatePrompt from '../ServiceWorkerUpdatePrompt';
import { Input } from '../ui/Input';
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

/** Put a field on the page holding `value`, as the page itself filled it in. */
function prefilled(
  tag: 'textarea' | 'input',
  value: string,
  type?: string,
): HTMLInputElement | HTMLTextAreaElement {
  const field = document.createElement(tag);
  if (type) {
    field.setAttribute('type', type);
  }
  field.value = value;
  document.body.appendChild(field);
  return field;
}

/** Put a field on the page as if the athlete had typed `value` into it. */
function typed(tag: 'textarea' | 'input', value: string, type?: string): HTMLElement {
  const field = prefilled(tag, '', type);
  fireEvent.input(field, { target: { value } });
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

  it('takes the build without asking over a value the page filled in', () => {
    sw.needRefresh = true;
    prefilled('input', 'Alice Martin', 'text');
    render(<ServiceWorkerUpdatePrompt />);
    expect(sw.updateServiceWorker).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('sw-update-prompt')).toBeNull();
  });

  it('asks once the athlete edits a value the page filled in', () => {
    sw.needRefresh = true;
    const name = prefilled('input', 'Alice Martin', 'text');
    fireEvent.input(name, { target: { value: 'Alice Martin-Roy' } });
    render(<ServiceWorkerUpdatePrompt />);
    expect(screen.getByTestId('sw-update-prompt')).toBeTruthy();
    expect(sw.updateServiceWorker).not.toHaveBeenCalled();
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
    document.body.appendChild(region);
    region.textContent = 'draft';
    fireEvent.input(region);
    expect(typedTextWouldBeLost()).toBe(true);
  });

  it('is false for values the page filled in that the athlete never edited', () => {
    prefilled('input', 'Alice Martin', 'text');
    prefilled('textarea', 'Saved bio');
    const region = document.createElement('div');
    region.setAttribute('contenteditable', 'true');
    region.textContent = 'Saved notes';
    document.body.appendChild(region);
    expect(typedTextWouldBeLost()).toBe(false);
  });

  it('is true once the athlete edits a value the page filled in', () => {
    const name = prefilled('input', 'Alice Martin', 'text');
    expect(typedTextWouldBeLost()).toBe(false);
    fireEvent.input(name, { target: { value: 'Alice' } });
    expect(typedTextWouldBeLost()).toBe(true);
  });

  it('is true for an empty composer the athlete typed into', async () => {
    const composer = prefilled('textarea', '');
    expect(typedTextWouldBeLost()).toBe(false);
    await userEvent.type(composer, 'Ma sortie');
    expect(typedTextWouldBeLost()).toBe(true);
  });

  it('is false again once the athlete clears what was typed', async () => {
    const composer = prefilled('textarea', '');
    await userEvent.type(composer, 'Ma sortie');
    expect(typedTextWouldBeLost()).toBe(true);
    await userEvent.clear(composer);
    expect(typedTextWouldBeLost()).toBe(false);
  });

  it('records the editable region when the edit lands in a node inside it', () => {
    const region = document.createElement('div');
    region.setAttribute('contenteditable', 'true');
    const paragraph = document.createElement('p');
    region.appendChild(paragraph);
    document.body.appendChild(region);
    paragraph.textContent = 'draft';
    fireEvent.input(paragraph);
    expect(typedTextWouldBeLost()).toBe(true);
  });

  it('is false for an edited field that has left the page', () => {
    const field = typed('textarea', 'draft');
    expect(typedTextWouldBeLost()).toBe(true);
    field.remove();
    expect(typedTextWouldBeLost()).toBe(false);
  });

  it('reads a React controlled field filled from saved state as untouched until typed into', async () => {
    function DisplayName() {
      const [name, setName] = useState('Alice Martin');
      return <Input aria-label="name" value={name} onChange={(e) => setName(e.target.value)} />;
    }
    const { getByLabelText, unmount } = render(<DisplayName />);
    expect(typedTextWouldBeLost()).toBe(false);
    await userEvent.type(getByLabelText('name'), ' Roy');
    expect(typedTextWouldBeLost()).toBe(true);
    unmount();
  });

  it('records the outermost editable region when regions are nested', () => {
    const outer = document.createElement('div');
    outer.setAttribute('contenteditable', 'true');
    const inner = document.createElement('div');
    inner.setAttribute('contenteditable', 'true');
    outer.appendChild(inner);
    document.body.appendChild(outer);
    inner.textContent = 'draft';
    fireEvent.input(inner);
    outer.removeChild(inner);
    outer.textContent = 'draft';
    expect(typedTextWouldBeLost()).toBe(true);
  });
});
