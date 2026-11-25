// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bring-your-own OAuth app modal's intro reads as one sentence in the app language
// ABOUTME: The developer-portal link sits inside the translated sentence, not glued between English halves; over the credential login it is the top layer

import { useState } from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import OAuthAppSetupModal from '../OAuthAppSetupModal';
import { Modal } from '../ui/Modal';

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

/**
 * The credential login with the OAuth setup opened over it. The login renders the
 * setup beside its own dialog container, never inside it, as here.
 */
function SetupOverLogin({ onLoginClose }: { onLoginClose: () => void }) {
  const [setup, setSetup] = useState(false);
  const [client] = useState(() => new QueryClient({ defaultOptions: { queries: { retry: false } } }));
  return (
    <QueryClientProvider client={client}>
      <Modal isOpen onClose={onLoginClose} title="Sign in to Strava">
        <button type="button" onClick={() => setSetup(true)}>
          Use my own app
        </button>
      </Modal>
      <OAuthAppSetupModal
        isOpen={setup}
        onClose={() => setSetup(false)}
        onSaved={vi.fn()}
        provider="strava"
        displayName="Strava"
        devPortalUrl="https://www.strava.com/settings/api"
      />
    </QueryClientProvider>
  );
}

describe('OAuthAppSetupModal over the credential login', () => {
  it('closes alone on one Escape and keeps Tab inside it', async () => {
    const user = userEvent.setup();
    const onLoginClose = vi.fn();
    render(<SetupOverLogin onLoginClose={onLoginClose} />);

    await user.click(screen.getByRole('button', { name: 'Use my own app' }));
    const setup = screen.getByRole('dialog', { name: 'Set up your Strava OAuth app' });
    // A full lap of Tab never leaves the form for the login under it.
    for (let step = 0; step < 10; step += 1) {
      await user.tab();
      expect(setup).toContainElement(document.activeElement as HTMLElement);
    }

    await user.keyboard('{Escape}');
    // carnet#1553: the login took the Escape and the setup closed with it.
    expect(screen.queryByRole('dialog', { name: 'Set up your Strava OAuth app' })).not.toBeInTheDocument();
    expect(onLoginClose).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Use my own app' })).toHaveFocus();

    await user.keyboard('{Escape}');
    expect(onLoginClose).toHaveBeenCalledTimes(1);
  });
});
