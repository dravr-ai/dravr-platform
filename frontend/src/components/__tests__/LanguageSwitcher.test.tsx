// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Covers the web language switcher — French by default, and one switch moving both locales
// ABOUTME: Asserts the real PUT /api/user/locale fires on a switch and when a stored choice disagrees with the account

import { StrictMode } from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { initI18n, useTranslation, LANGUAGE_STORAGE_KEY } from '@pierre/i18n';

import { LanguageSwitcher } from '../LanguageSwitcher';
import { persistLocale } from '../../i18n/localePersister';
import { pierreApi } from '../../services/api';

/** Renders a real translated string next to the switcher, so "chrome changed" is observable. */
function Harness({ serverLocale }: { serverLocale?: string }) {
  const { t } = useTranslation();
  return (
    <div>
      <p data-testid="chrome">{t('settings.language')}</p>
      <LanguageSwitcher serverLocale={serverLocale} />
    </div>
  );
}

beforeEach(async () => {
  localStorage.clear();
  // The production persister, not a stand-in: the test asserts the request it
  // actually issues.
  await initI18n({ persistLocale, config: { lng: 'fr' } });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('LanguageSwitcher — one switch, two locales', () => {
  it('renders French for a client with no stored preference', () => {
    render(<Harness />);

    expect(screen.getByTestId('chrome').textContent).toBe('Langue');
    expect(screen.getByLabelText('Choisir la langue')).toHaveValue('fr');
  });

  it('restores a stored preference under StrictMode double-mounting', async () => {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, 'es');

    render(
      <StrictMode>
        <Harness />
      </StrictMode>,
    );

    // StrictMode mounts, tears down and remounts every effect. A restore
    // guarded by a "ran once" ref applies nothing on the second mount, and the
    // viewer's remembered language is silently dropped.
    await waitFor(() => {
      expect(screen.getByTestId('chrome').textContent).toBe('Idioma');
    });
    expect(screen.getByLabelText('Elegir idioma')).toHaveValue('es');
  });

  it('adopts the account locale when this browser has no stored choice', async () => {
    render(<Harness serverLocale="de" />);

    await waitFor(() => {
      expect(screen.getByTestId('chrome').textContent).toBe('Sprache');
    });
    expect(screen.getByLabelText('Sprache wählen')).toHaveValue('de');
  });

  it('changes the chrome AND PUTs the chosen locale to the server', async () => {
    const put = vi
      .spyOn(pierreApi.axios, 'put')
      .mockResolvedValue({ data: { message: 'Locale updated', locale: 'de' } });

    render(<Harness />);
    await userEvent.selectOptions(screen.getByLabelText('Choisir la langue'), 'de');

    await waitFor(() => {
      expect(put).toHaveBeenCalledTimes(1);
    });
    expect(put).toHaveBeenCalledWith('/api/user/locale', { locale: 'de' });
    expect(screen.getByTestId('chrome').textContent).toBe('Sprache');
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('tells the user when the chrome moved but the server write failed', async () => {
    vi.spyOn(pierreApi.axios, 'put').mockRejectedValue(new Error('offline'));

    render(<Harness />);
    await userEvent.selectOptions(screen.getByLabelText('Choisir la langue'), 'pt');

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toBe(
      'O idioma da interface mudou, mas o das respostas do teu agente não pôde ser guardado. Tenta de novo.',
    );
  });
});

describe('LanguageSwitcher — a stored choice reaches the account on mount', () => {
  it('PUTs the stored locale when the signed-in account disagrees', async () => {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, 'es');
    const put = vi
      .spyOn(pierreApi.axios, 'put')
      .mockResolvedValue({ data: { message: 'Locale updated', locale: 'es' } });

    render(<Harness serverLocale="de" />);

    await waitFor(() => {
      expect(put).toHaveBeenCalledWith('/api/user/locale', { locale: 'es' });
    });
    expect(put).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('chrome').textContent).toBe('Idioma');
    await waitFor(() => {
      expect(screen.getByLabelText('Elegir idioma')).toBeEnabled();
    });
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('does not PUT when the stored locale already matches the account', async () => {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, 'de');
    const put = vi.spyOn(pierreApi.axios, 'put');

    render(<Harness serverLocale="de" />);

    await waitFor(() => {
      expect(screen.getByTestId('chrome').textContent).toBe('Sprache');
    });
    expect(put).not.toHaveBeenCalled();
  });

  it('does not PUT when nobody is signed in', async () => {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, 'es');
    const put = vi.spyOn(pierreApi.axios, 'put');

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByTestId('chrome').textContent).toBe('Idioma');
    });
    expect(put).not.toHaveBeenCalled();
  });

  it('tells the user when the write-back failed', async () => {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, 'pt');
    vi.spyOn(pierreApi.axios, 'put').mockRejectedValue(new Error('offline'));

    render(<Harness serverLocale="en" />);

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toBe(
      'O idioma da interface mudou, mas o das respostas do teu agente não pôde ser guardado. Tenta de novo.',
    );
  });
});
