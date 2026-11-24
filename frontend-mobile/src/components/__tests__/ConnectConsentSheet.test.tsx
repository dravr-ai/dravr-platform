// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The consent sheet before a Garmin connect — both boxes start unticked, Continue is never held, the ticked keys come back
// ABOUTME: Pins the empty answer, both keys, the reset on reopening, Cancel, and the privacy-policy link (carnet#1157, carnet#1390)

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import { GARMIN_PRIVACY_URL, connectConsentsFor } from '@pierre/shared-constants';
import { ConnectConsentSheet } from '../ConnectConsentSheet';
import { openExternal } from '../../utils/openExternal';

jest.mock('../../utils/openExternal', () => ({ openExternal: jest.fn() }));

const GARMIN = connectConsentsFor('sciotte_garmin');

function renderSheet(visible = true) {
  const onContinue = jest.fn();
  const onCancel = jest.fn();
  const view = render(
    <ConnectConsentSheet visible={visible} consents={GARMIN} onCancel={onCancel} onContinue={onContinue} />,
  );
  return { onContinue, onCancel, view };
}

describe('ConnectConsentSheet', () => {
  beforeEach(() => jest.clearAllMocks());

  it('offers both Garmin boxes, unticked, with Continue enabled', () => {
    renderSheet();
    expect(GARMIN.map((consent) => consent.key)).toEqual(['garmin', 'garmin_workout_transfer']);
    expect(screen.getByTestId('connect-consent-box-garmin')).not.toBeChecked();
    expect(screen.getByTestId('connect-consent-box-garmin_workout_transfer')).not.toBeChecked();
    expect(screen.getByTestId('connect-consent-continue')).toBeEnabled();
  });

  it('continues with no key when neither box is ticked', () => {
    const { onContinue } = renderSheet();
    fireEvent.press(screen.getByTestId('connect-consent-continue'));
    expect(onContinue).toHaveBeenCalledWith([]);
  });

  it('continues with both keys when both boxes are ticked, and with one once the other is unticked', () => {
    const { onContinue } = renderSheet();
    fireEvent.press(screen.getByTestId('connect-consent-box-garmin_workout_transfer'));
    fireEvent.press(screen.getByTestId('connect-consent-box-garmin'));
    expect(screen.getByTestId('connect-consent-box-garmin')).toBeChecked();
    fireEvent.press(screen.getByTestId('connect-consent-continue'));
    expect(onContinue).toHaveBeenLastCalledWith(['garmin', 'garmin_workout_transfer']);

    fireEvent.press(screen.getByTestId('connect-consent-box-garmin'));
    fireEvent.press(screen.getByTestId('connect-consent-continue'));
    expect(onContinue).toHaveBeenLastCalledWith(['garmin_workout_transfer']);
  });

  it('starts unticked again on every opening', () => {
    const { onContinue, onCancel, view } = renderSheet();
    fireEvent.press(screen.getByTestId('connect-consent-box-garmin'));
    view.rerender(<ConnectConsentSheet visible={false} consents={GARMIN} onCancel={onCancel} onContinue={onContinue} />);
    view.rerender(<ConnectConsentSheet visible consents={GARMIN} onCancel={onCancel} onContinue={onContinue} />);
    expect(screen.getByTestId('connect-consent-box-garmin')).not.toBeChecked();
    fireEvent.press(screen.getByTestId('connect-consent-continue'));
    expect(onContinue).toHaveBeenCalledWith([]);
  });

  it('cancels without continuing', () => {
    const { onContinue, onCancel } = renderSheet();
    fireEvent.press(screen.getByTestId('connect-consent-cancel'));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onContinue).not.toHaveBeenCalled();
  });

  it("links the transfer box, and only it, to Garmin's privacy policy", () => {
    renderSheet();
    expect(screen.queryByTestId('connect-consent-link-garmin')).toBeNull();
    fireEvent.press(screen.getByTestId('connect-consent-link-garmin_workout_transfer'));
    expect(openExternal).toHaveBeenCalledWith(GARMIN_PRIVACY_URL, expect.any(Function));
  });
});
