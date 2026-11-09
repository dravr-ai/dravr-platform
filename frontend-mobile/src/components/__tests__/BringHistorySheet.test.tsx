// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile "Bring in your history" sheet lists a card's history choices and opens the next window only once it has gone
// ABOUTME: Pins the sign-in and export choices, the export hidden without an offered or copied source, its route, and Not now

import React from 'react';
import { Modal, Platform } from 'react-native';
import { act, fireEvent, render, screen } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import type { HistoryState } from '@pierre/shared-types';
import { BringHistorySheet } from '../BringHistorySheet';
import type { ExtendedProviderStatus } from '../../types';

const mockPush = jest.fn();
jest.mock('expo-router', () =>
  require('../../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
  }),
);

/** The Garmin card once its grant landed, its history at `state`. */
function garmin(state: HistoryState, archive_source: string | null = 'garmin_archive'): ExtendedProviderStatus {
  return {
    provider: 'sciotte_garmin',
    display_name: 'Garmin',
    description: '',
    requires_oauth: false,
    connected: true,
    connected_backend: 'garmin',
    needs_reauth: false,
    capabilities: ['activities'],
    consent_required: false,
    own_app_required: false,
    recommended_backend: 'oauth',
    history: { state, sign_in_target: 'sciotte_garmin', archive_source },
  } as ExtendedProviderStatus;
}

/** iOS reports the sheet gone through the panel's `onDismiss`; a test sends it. */
function sheetGone() {
  const modal = screen.UNSAFE_getAllByType(Modal).find((each) => typeof each.props.onDismiss === 'function');
  if (!modal) throw new Error('the history sheet has no onDismiss');
  act(() => modal.props.onDismiss());
}

function renderSheet(card: ExtendedProviderStatus | null, props: { onboarding?: boolean } = {}) {
  const onSignIn = jest.fn();
  const onClose = jest.fn();
  const view = render(<BringHistorySheet card={card} onSignIn={onSignIn} onClose={onClose} {...props} />);
  return { ...view, onSignIn, onClose };
}

describe('BringHistorySheet (mobile)', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('names the provider and lists the sign-in and the export', () => {
    renderSheet(garmin('needed'));

    expect(screen.getByText(i18n.t('providers.history.sheetTitle', { provider: 'Garmin' }))).toBeTruthy();
    expect(screen.getByText(i18n.t('providers.history.sheetBody', { provider: 'Garmin' }))).toBeTruthy();
    expect(screen.getByTestId('history-choice-sign-in')).toHaveTextContent(
      `${i18n.t('providers.history.choiceSignIn')}${i18n.t('providers.history.explainer', { provider: 'Garmin' })}`,
    );
    expect(screen.getByTestId('history-choice-archive')).toHaveTextContent(
      `${i18n.t('providers.history.choiceArchive', { provider: 'Garmin' })}${i18n.t('providers.history.choiceArchiveHint', { provider: 'Garmin' })}`,
    );
    expect(screen.getByText(i18n.t('providers.history.notNow'))).toBeTruthy();
  });

  it('leaves the export out when the server offers no source', () => {
    renderSheet(garmin('needed', null));

    expect(screen.getByTestId('history-choice-sign-in')).toBeTruthy();
    expect(screen.queryByTestId('history-choice-archive')).toBeNull();
  });

  it('leaves the export out when this build carries no copy for the source', () => {
    renderSheet(garmin('needed', 'some_future_archive'));

    expect(screen.queryByTestId('history-choice-archive')).toBeNull();
  });

  it('offers only the export once the copy is in', () => {
    renderSheet(garmin('imported'));

    expect(screen.queryByTestId('history-choice-sign-in')).toBeNull();
    expect(screen.getByTestId('history-choice-archive')).toBeTruthy();
  });

  it('closes first and opens the Garmin sign-in only once the sheet has gone', () => {
    const card = garmin('needed');
    const { onSignIn, onClose, rerender } = renderSheet(card);

    fireEvent.press(screen.getByTestId('history-choice-sign-in'));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSignIn).not.toHaveBeenCalled();

    rerender(<BringHistorySheet card={null} onSignIn={onSignIn} onClose={onClose} />);
    sheetGone();
    expect(onSignIn).toHaveBeenCalledWith('garmin', card);
  });

  it('opens the sign-in at once on Android, which stacks the windows', () => {
    const os = Platform.OS;
    Object.defineProperty(Platform, 'OS', { value: 'android', configurable: true });
    try {
      const card = garmin('incomplete');
      const { onSignIn, onClose } = renderSheet(card);

      fireEvent.press(screen.getByTestId('history-choice-sign-in'));
      expect(onClose).toHaveBeenCalledTimes(1);
      expect(onSignIn).toHaveBeenCalledWith('garmin', card);
    } finally {
      Object.defineProperty(Platform, 'OS', { value: os, configurable: true });
    }
  });

  it("opens the export's import over the app, on that source", () => {
    const { onClose } = renderSheet(garmin('needed'));

    fireEvent.press(screen.getByTestId('history-choice-archive'));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(mockPush).not.toHaveBeenCalled();
    sheetGone();
    expect(mockPush).toHaveBeenCalledWith({ pathname: '/(app)/import', params: { source: 'garmin_archive' } });
  });

  it("opens the export's import inside onboarding's connect step", () => {
    renderSheet(garmin('needed'), { onboarding: true });

    fireEvent.press(screen.getByTestId('history-choice-archive'));
    sheetGone();
    expect(mockPush).toHaveBeenCalledWith({ pathname: '/(onboarding)/import', params: { source: 'garmin_archive' } });
  });

  it('closes on Not now and opens nothing', () => {
    const { onSignIn, onClose } = renderSheet(garmin('needed'));

    fireEvent.press(screen.getByTestId('history-not-now'));
    sheetGone();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSignIn).not.toHaveBeenCalled();
    expect(mockPush).not.toHaveBeenCalled();
  });

  it('draws nothing while it has no card', () => {
    renderSheet(null);

    expect(screen.queryByTestId('history-sheet')).toBeNull();
  });
});
