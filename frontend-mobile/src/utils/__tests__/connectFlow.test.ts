// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the one refusal step Connections and onboarding's connect step share
// ABOUTME: A refusal that changed the row's plan reads the rows again and signs in; any other marks the row and reads nothing

import { showConnectRefusal } from '../connectFlow';
import type { ExtendedProviderStatus } from '../../types';

function garmin(recommended_backend: 'oauth' | 'mirror'): ExtendedProviderStatus {
  return {
    provider: 'sciotte_garmin',
    display_name: 'Garmin',
    requires_oauth: false,
    connected: false,
    needs_reauth: false,
    capabilities: ['activities'],
    recommended_backend,
    seats_left: recommended_backend === 'oauth' ? 3 : 0,
  } as ExtendedProviderStatus;
}

function surface(served: ExtendedProviderStatus[]) {
  return {
    reloadCards: jest.fn().mockResolvedValue(served),
    openSignIn: jest.fn(),
    setRefusedCard: jest.fn(),
  };
}

describe('showConnectRefusal', () => {
  it('marks the row and reads nothing when the refusal does not ask for a reload', async () => {
    const s = surface([garmin('mirror')]);
    await showConnectRefusal('sciotte_garmin', { reload: false }, s);
    expect(s.reloadCards).not.toHaveBeenCalled();
    expect(s.openSignIn).not.toHaveBeenCalled();
    expect(s.setRefusedCard).toHaveBeenCalledWith({ card: 'sciotte_garmin' });
  });

  it('reads the rows again and opens the sign-in of a row that now signs in', async () => {
    const card = garmin('mirror');
    const s = surface([card]);
    await showConnectRefusal('sciotte_garmin', { reload: true }, s);
    expect(s.reloadCards).toHaveBeenCalledTimes(1);
    expect(s.openSignIn).toHaveBeenCalledWith('garmin', card, undefined);
    expect(s.setRefusedCard).not.toHaveBeenCalled();
  });

  it('re-opens the sign-in with the consents ticked before the refused connect', async () => {
    const card = garmin('mirror');
    const s = surface([card]);
    const consents = ['garmin_ai_use'];
    await showConnectRefusal('sciotte_garmin', { reload: true, consents }, s);
    expect(s.openSignIn).toHaveBeenCalledWith('garmin', card, consents);
    expect(s.setRefusedCard).not.toHaveBeenCalled();
  });

  it('marks the row when the rows read again still authorize, or could not be read', async () => {
    const still = surface([garmin('oauth')]);
    await showConnectRefusal('sciotte_garmin', { reload: true }, still);
    expect(still.openSignIn).not.toHaveBeenCalled();
    expect(still.setRefusedCard).toHaveBeenCalledWith({ card: 'sciotte_garmin' });

    const unread = surface([]);
    await showConnectRefusal('sciotte_garmin', { reload: true }, unread);
    expect(unread.setRefusedCard).toHaveBeenCalledWith({ card: 'sciotte_garmin' });
  });
});
