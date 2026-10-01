// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bring-your-own OAuth app modal's intro reads as one sentence in the app language
// ABOUTME: The developer-portal link sits inside the translated sentence, not glued between English halves

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import OAuthAppSetupModal from '../OAuthAppSetupModal';

vi.mock('../../services/api', () => ({
  userApi: {
    getOAuthApps: vi.fn().mockResolvedValue({ apps: [] }),
    registerOAuthApp: vi.fn(),
  },
}));

function renderModal() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <OAuthAppSetupModal
        isOpen
        onClose={vi.fn()}
        onSaved={vi.fn()}
        provider="whoop"
        displayName="WHOOP"
        devPortalUrl="https://developer-dashboard.whoop.com/apps"
      />
    </QueryClientProvider>,
  );
}

/** The intro paragraph, read the way a person reads it: all of its text, link included. */
function introParagraph(): HTMLElement {
  const link = screen.getByRole('link', { name: 'developer-dashboard.whoop.com' });
  const paragraph = link.closest('p');
  if (paragraph === null) {
    throw new Error('the developer-portal link is not inside the intro paragraph');
  }
  return paragraph;
}

describe('OAuthAppSetupModal intro', () => {
  it('says the whole sentence in English, link inside it', () => {
    renderModal();

    expect(introParagraph().textContent).toBe(
      'WHOOP requires each user to register their own developer app. Create one at ' +
        'developer-dashboard.whoop.com and use the redirect URI shown below.',
    );
    expect(screen.getByRole('link', { name: 'developer-dashboard.whoop.com' })).toHaveAttribute(
      'href',
      'https://developer-dashboard.whoop.com/apps',
    );
  });

  it('says the whole sentence in French, link inside it', async () => {
    await i18n.changeLanguage('fr');
    try {
      renderModal();

      expect(introParagraph().textContent).toBe(
        'WHOOP demande à chaque utilisateur d’enregistrer sa propre application de développeur. ' +
          'Crées-en une sur developer-dashboard.whoop.com et utilise l’URI de redirection ' +
          'indiquée ci-dessous.',
      );
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
