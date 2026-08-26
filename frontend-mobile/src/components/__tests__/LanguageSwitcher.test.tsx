// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Covers the mobile language switcher — French by default, and one tap moving both locales
// ABOUTME: Asserts the real PUT /api/user/locale fires, so chrome and reply language cannot drift apart

import React from 'react';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { render, screen, waitFor, fireEvent } from '@testing-library/react-native';
import { Text, View } from 'react-native';
import { initI18n, useTranslation, LANGUAGE_STORAGE_KEY } from '@pierre/i18n';

import { LanguageSwitcher } from '../LanguageSwitcher';
import { persistLocale } from '../../i18n/localePersister';

// The real `userApi`, built by the real factory, over a stubbed axios. That
// keeps the assertion on the request `updateLocale` actually issues — path and
// body — while leaving the network client (which reads Expo env at import
// time) out of the test process.
const mockPut = jest.fn();
jest.mock('../../services/api', () => {
  const { createUserApi } = jest.requireActual('@pierre/api-client');
  // The factory runs while `mockPut` is still in its temporal dead zone, so
  // the stub forwards through a closure rather than capturing the value.
  return {
    userApi: createUserApi({
      put: (...args: unknown[]) => mockPut(...args),
    }),
  };
});

/** Renders a real translated string next to the switcher, so "chrome changed" is observable. */
function Harness({ serverLocale }: { serverLocale?: string }) {
  const { t } = useTranslation();
  return (
    <View>
      <Text testID="chrome">{t('settings.language')}</Text>
      <LanguageSwitcher serverLocale={serverLocale} />
    </View>
  );
}

beforeEach(async () => {
  await AsyncStorage.clear();
  // The production persister, not a stand-in: the test asserts the request it
  // actually issues.
  await initI18n({ persistLocale, config: { lng: 'fr' } });
});

afterEach(() => {
  mockPut.mockReset();
});

describe('LanguageSwitcher — one tap, two locales', () => {
  it('renders French for a device with no stored preference', () => {
    render(<Harness />);

    expect(screen.getByTestId('chrome')).toHaveTextContent('Langue');
    expect(screen.getByTestId('language-option-fr').props.accessibilityState.selected).toBe(true);
  });

  it('adopts the account locale when this device has no stored choice', async () => {
    render(<Harness serverLocale="pt" />);

    await waitFor(() => {
      expect(screen.getByTestId('chrome')).toHaveTextContent('Idioma');
    });
    expect(screen.getByTestId('language-option-pt').props.accessibilityState.selected).toBe(true);
  });

  it('changes the chrome AND PUTs the chosen locale to the server', async () => {
    mockPut.mockResolvedValue({ data: { message: 'Locale updated', locale: 'de' } });

    render(<Harness />);
    fireEvent.press(screen.getByTestId('language-option-de'));

    await waitFor(() => {
      expect(mockPut).toHaveBeenCalledTimes(1);
    });
    expect(mockPut).toHaveBeenCalledWith('/api/user/locale', { locale: 'de' });
    expect(screen.getByTestId('chrome')).toHaveTextContent('Sprache');
    expect(await AsyncStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
    expect(screen.queryByTestId('language-sync-error')).toBeNull();
  });

  it('tells the user when the chrome moved but the server write failed', async () => {
    mockPut.mockRejectedValue(new Error('offline'));

    render(<Harness />);
    fireEvent.press(screen.getByTestId('language-option-es'));

    const alert = await screen.findByTestId('language-sync-error');
    expect(alert).toHaveTextContent(
      'El idioma de la interfaz cambió, pero no se pudo guardar el de las respuestas de tu coach. Inténtalo de nuevo.',
    );
  });
});
