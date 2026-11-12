// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The state of the provider sign-in window a screen mounts — its account, the notice it owes, history mode, its OAuth fallback
// ABOUTME: One hook for Connections, onboarding's connect step and a chat thread, so each opens the window the same way

import { useCallback, useState } from 'react';
import { SCIOTTE_LOGIN_PRESETS, oauthFallbackTarget } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import type { ExtendedProviderStatus } from '../types';

export interface SignInModalState {
  /** The account the window signs in to; `null` while it is closed. */
  target: SciotteTarget | null;
  /** The card has not accepted its provider's notice, so the window asks for it first. */
  consentRequired: boolean;
  /** The sign-in copies the history of a card already connected another way. */
  historyMode: boolean;
  /**
   * The provider whose OAuth a sign-in that cannot complete falls back to
   * (`oauthFallbackTarget`, carnet#1477), or `null`: set only for a sign-in a
   * card's plan opened, never one a refused grant or a history copy opened.
   */
  fallback: SciotteTarget | null;
}

const CLOSED: SignInModalState = { target: null, consentRequired: false, historyMode: false, fallback: null };

/** The card a sign-in is opened for: its notice, and what its OAuth fallback reads. */
type SignInCard = Pick<
  ExtendedProviderStatus,
  'consent_required' | 'provider' | 'connected' | 'recommended_backend' | 'seats_left'
>;

export interface SignInModal extends SignInModalState {
  /**
   * Open the sign-in to `target` for `card`, whose notice it owes while the
   * card says so. A card connected through its provider's authorization
   * passes `historyMode`: the sign-in then copies the history once. A sign-in
   * the card's own plan opens passes `fromPlan`, so one that cannot complete
   * falls back to the card's OAuth when it offers one.
   */
  open: (target: SciotteTarget, card: SignInCard | undefined, historyMode?: boolean, fromPlan?: boolean) => void;
  /** Open the sign-in to `target` with the notice owed by its card among `cards`. */
  openFor: (target: SciotteTarget, cards: readonly ExtendedProviderStatus[]) => void;
  close: () => void;
}

export function useSignInModal(): SignInModal {
  const [state, setState] = useState<SignInModalState>(CLOSED);
  const open = useCallback<SignInModal['open']>((target, card, historyMode = false, fromPlan = false) => {
    const fallback = fromPlan && !historyMode && card ? oauthFallbackTarget(card) : null;
    setState({ target, consentRequired: card?.consent_required ?? false, historyMode, fallback });
  }, []);
  const openFor = useCallback<SignInModal['openFor']>(
    (target, cards) => open(target, cards.find((card) => card.provider === SCIOTTE_LOGIN_PRESETS[target].backend)),
    [open],
  );
  const close = useCallback(() => setState(CLOSED), []);
  return { ...state, open, openFor, close };
}
