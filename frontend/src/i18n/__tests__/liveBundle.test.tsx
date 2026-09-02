// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The live catalogue overlay — nesting, repaint on apply, ETag reuse, fail-open on a rejected fetch
// ABOUTME: Asserts what is on screen after an overlay, never that a call returned

import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import {
  i18n,
  useTranslation,
  nestDotted,
  applyLiveBundle,
  installLiveOverlay,
  type BundleFetcher,
} from '@pierre/i18n';

/** Renders one catalogue key, so an overlay is judged by the pixels it changes. */
function Probe() {
  const { t } = useTranslation();
  return <span data-testid="probe">{t('common.cancel')}</span>;
}

const ORIGINAL_CANCEL = i18n.t('common.cancel', { lng: 'en' });

function restoreCancel() {
  i18n.addResourceBundle('en', 'translation', { common: { cancel: ORIGINAL_CANCEL } }, true, true);
}

afterEach(async () => {
  restoreCancel();
  if (i18n.language !== 'en') {
    await i18n.changeLanguage('en');
  }
});

describe('nestDotted', () => {
  it('nests dotted keys into the shape i18next resolves', () => {
    expect(nestDotted({ 'a.b.c': 'x', 'a.d': 'y', e: 'z' })).toEqual({
      a: { b: { c: 'x' }, d: 'y' },
      e: 'z',
    });
  });

  it('refuses a key that nests under a leaf or lands on a subtree', () => {
    expect(() => nestDotted({ a: 'leaf', 'a.b': 'x' })).toThrow(/nests under the leaf a/);
    expect(() => nestDotted({ 'a.b': 'x', a: 'leaf' })).toThrow(/already a subtree/);
  });
});

describe('applyLiveBundle', () => {
  it('repaints mounted chrome with the live text', async () => {
    render(<Probe />);
    expect(screen.getByTestId('probe').textContent).toBe(ORIGINAL_CANCEL);

    act(() => {
      applyLiveBundle(i18n, {
        locale: 'en',
        etag: 'e1',
        strings: { 'common.cancel': 'Abort (live)' },
      });
    });

    expect(await screen.findByText('Abort (live)')).toBeTruthy();
  });

  it('keeps embedded text for keys the bundle lacks', () => {
    const originalSave = i18n.t('common.save', { lng: 'en' });
    applyLiveBundle(i18n, { locale: 'en', etag: 'e2', strings: { 'common.save': 'Keep (live)' } });
    expect(i18n.t('common.save')).toBe('Keep (live)');
    expect(i18n.t('common.cancel')).toBe(ORIGINAL_CANCEL);
    i18n.addResourceBundle('en', 'translation', { common: { save: originalSave } }, true, true);
  });
});

describe('installLiveOverlay', () => {
  it('fetches the current locale on install and sends the digest back on the next refresh', async () => {
    const fetcher = vi.fn<BundleFetcher>(async (locale, etag) =>
      etag === 'e1'
        ? { status: 'unchanged' }
        : { status: 'fresh', bundle: { locale, etag: 'e1', strings: { 'common.cancel': 'Once (live)' } } },
    );

    const overlay = installLiveOverlay(i18n, fetcher);
    await overlay.refresh('en');
    expect(fetcher).toHaveBeenCalledWith('en', undefined);
    expect(i18n.t('common.cancel')).toBe('Once (live)');

    await overlay.refresh('en');
    expect(fetcher).toHaveBeenLastCalledWith('en', 'e1');
    expect(i18n.t('common.cancel')).toBe('Once (live)');
    overlay.dispose();
  });

  it('refreshes the new locale when the language changes', async () => {
    const fetcher = vi.fn<BundleFetcher>(async () => ({ status: 'unchanged' }));
    const overlay = installLiveOverlay(i18n, fetcher);
    await act(async () => {
      await i18n.changeLanguage('fr');
    });
    expect(fetcher).toHaveBeenCalledWith('fr', undefined);
    overlay.dispose();
  });

  it('leaves the embedded copy on screen when the fetch fails', async () => {
    const fetcher = vi.fn<BundleFetcher>(async () => {
      throw new Error('offline');
    });
    render(<Probe />);
    const overlay = installLiveOverlay(i18n, fetcher);
    await overlay.refresh('en');
    expect(screen.getByTestId('probe').textContent).toBe(ORIGINAL_CANCEL);
    overlay.dispose();
  });
});
