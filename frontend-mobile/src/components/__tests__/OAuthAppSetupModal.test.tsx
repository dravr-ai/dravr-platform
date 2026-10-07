// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's bring-your-own OAuth app sheet opens on one sentence in the app language
// ABOUTME: The developer-portal host is a pressable span inside the translated sentence, not glued between English halves

import React from 'react';
import { Alert, Linking } from 'react-native';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import { OAuthAppSetupModal } from '../OAuthAppSetupModal';

jest.mock('../../services/api', () => ({
  userApi: {
    getOAuthApps: jest.fn().mockResolvedValue({ apps: [] }),
    registerOAuthApp: jest.fn(),
  },
}));

function renderSheet() {
  return render(
    <OAuthAppSetupModal
      visible
      onClose={jest.fn()}
      onSaved={jest.fn()}
      provider="whoop"
      displayName="WHOOP"
      devPortalUrl="https://developer-dashboard.whoop.com/apps"
    />,
  );
}

describe('OAuthAppSetupModal (mobile) intro', () => {
  it('says the whole sentence in English and opens the portal from the host inside it', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    renderSheet();

    expect(
      await screen.findByText(
        'WHOOP requires each user to register their own developer app. Create one at ' +
          'developer-dashboard.whoop.com and use the redirect URI shown below.',
      ),
    ).toBeTruthy();

    fireEvent.press(screen.getByText('developer-dashboard.whoop.com'));
    await waitFor(() =>
      expect(openURL).toHaveBeenCalledWith('https://developer-dashboard.whoop.com/apps'),
    );
    openURL.mockRestore();
  });

  // carnet#803: the portal opens through openExternal, keeping this sheet's
  // own failure copy — it names the portal to open by hand.
  it('names the portal to open by hand when the device cannot open it', async () => {
    const openURL = jest.spyOn(Linking, 'openURL').mockRejectedValue(new Error('no handler'));
    const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    renderSheet();

    fireEvent.press(await screen.findByText('developer-dashboard.whoop.com'));

    await waitFor(() =>
      expect(alert).toHaveBeenCalledWith(
        i18n.t('app.unableOpenBrowser'),
        i18n.t('app.openPortalManually', {
          url: 'https://developer-dashboard.whoop.com/apps',
          provider: 'WHOOP',
        }),
      ),
    );
    alert.mockRestore();
    openURL.mockRestore();
  });

  it('says the whole sentence in French', async () => {
    await i18n.changeLanguage('fr');
    try {
      renderSheet();

      expect(
        await screen.findByText(
          'WHOOP demande à chaque utilisateur d’enregistrer sa propre application de développeur. ' +
            'Crées-en une sur developer-dashboard.whoop.com et utilise l’URI de redirection ' +
            'indiquée ci-dessous.',
        ),
      ).toBeTruthy();
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
