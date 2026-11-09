// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The state of the provider sign-in window a screen mounts — which account it signs in to, the notice it owes, history mode
// ABOUTME: One hook for Connections, onboarding's connect step and a chat thread, so each opens the window the same way

import { useCallback, useState } from 'react';
import { SCIOTTE_LOGIN_PRESETS } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import type { ExtendedProviderStatus } from '../types';

export interface SignInModalState {
  /** The account the window signs in to; `null` while it is closed. */
  target: SciotteTarget | null;
  /** The card has not accepted its provider's notice, so the window asks for it first. */
  consentRequired: boolean;
  /** The sign-in copies the history of a card already connected another way. */
  historyMode: boolean;
}

const CLOSED: SignInModalState = { target: null, consentRequired: false, historyMode: false };

export interface SignInModal extends SignInModalState {
  /**
   * Open the sign-in to `target` for `card`, whose notice it owes while the
   * card says so. A card connected through its provider's authorization
   * passes `historyMode`: the sign-in then copies the history once.
   */
  open: (target: SciotteTarget, card: Pick<ExtendedProviderStatus, 'consent_required'> | undefined, historyMode?: boolean) => void;
  /** Open the sign-in to `target` with the notice owed by its card among `cards`. */
  openFor: (target: SciotteTarget, cards: readonly ExtendedProviderStatus[]) => void;
  close: () => void;
}

export function useSignInModal(): SignInModal {
  const [state, setState] = useState<SignInModalState>(CLOSED);
  const open = useCallback<SignInModal['open']>((target, card, historyMode = false) => {
    setState({ target, consentRequired: card?.consent_required ?? false, historyMode });
  }, []);
  const openFor = useCallback<SignInModal['openFor']>(
    (target, cards) => open(target, cards.find((card) => card.provider === SCIOTTE_LOGIN_PRESETS[target].backend)),
    [open],
  );
  const close = useCallback(() => setState(CLOSED), []);
  return { ...state, open, openFor, close };
}
