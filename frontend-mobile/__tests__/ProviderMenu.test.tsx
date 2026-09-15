// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the provider row menu — reconnect when the token lapsed, disconnect as the destructive row, cancel
// ABOUTME: Covers the platform menu on both OSes, the row each index runs, the provider name as title, and the haptic on open

import { ActionSheetIOS, Alert, Platform } from 'react-native';
import * as Haptics from 'expo-haptics';
import { i18n } from '@pierre/i18n';

import { presentProviderMenu } from '../src/screens/connections/presentProviderMenu';

type SheetOptions = {
  title?: string;
  options: string[];
  cancelButtonIndex?: number;
  destructiveButtonIndex?: number;
};
type SheetCallback = (index: number) => void;
type AlertButton = { text?: string; onPress?: () => void; style?: string };

/** The rows the platform sheet last offered, and a way to pick one by index. */
function presentedSheet() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [options, callback] = spy.mock.calls[spy.mock.calls.length - 1] as [SheetOptions, SheetCallback];
  return {
    title: options.title,
    labels: options.options,
    cancelButtonIndex: options.cancelButtonIndex,
    destructiveButtonIndex: options.destructiveButtonIndex,
    pick: callback,
  };
}

/** The dialog Android last showed: its title and its buttons. */
function presentedDialog() {
  const spy = Alert.alert as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [title, , buttons] = spy.mock.calls[spy.mock.calls.length - 1] as [string, undefined, AlertButton[]];
  return { title, buttons };
}

function callbacks() {
  return { onReconnect: jest.fn(), onDisconnect: jest.fn() };
}

describe('the provider menu', () => {
  const originalOS = Platform.OS;

  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => {
    (Platform as { OS: string }).OS = originalOS;
    jest.restoreAllMocks();
  });

  describe('on iOS', () => {
    beforeEach(() => {
      (Platform as { OS: string }).OS = 'ios';
    });

    it('offers reconnect, disconnect and cancel under the provider name when the token lapsed', () => {
      presentProviderMenu({ providerName: 'Strava', canReconnect: true, ...callbacks() }, i18n.t);

      const sheet = presentedSheet();
      expect(sheet.title).toBe('Strava');
      expect(sheet.labels).toEqual(['Reconnect', 'Disconnect', 'Cancel']);
      expect(sheet.cancelButtonIndex).toBe(2);
      expect(sheet.destructiveButtonIndex).toBe(1);
    });

    it('offers only disconnect and cancel on a live connection', () => {
      presentProviderMenu({ providerName: 'Garmin', canReconnect: false, ...callbacks() }, i18n.t);

      const sheet = presentedSheet();
      expect(sheet.labels).toEqual(['Disconnect', 'Cancel']);
      expect(sheet.cancelButtonIndex).toBe(1);
      expect(sheet.destructiveButtonIndex).toBe(0);
    });

    it('each row runs its own callback, and cancel runs none', () => {
      const cb = callbacks();
      presentProviderMenu({ providerName: 'Strava', canReconnect: true, ...cb }, i18n.t);
      const sheet = presentedSheet();

      sheet.pick(0);
      expect(cb.onReconnect).toHaveBeenCalledTimes(1);
      expect(cb.onDisconnect).not.toHaveBeenCalled();
      sheet.pick(1);
      expect(cb.onDisconnect).toHaveBeenCalledTimes(1);

      sheet.pick(2);
      expect(cb.onReconnect).toHaveBeenCalledTimes(1);
      expect(cb.onDisconnect).toHaveBeenCalledTimes(1);
    });

    // Without the reconnect row the disconnect row moves up to index 0; the
    // callback must follow the rows, not a fixed position.
    it('disconnect is the first row on a live connection', () => {
      const cb = callbacks();
      presentProviderMenu({ providerName: 'Garmin', canReconnect: false, ...cb }, i18n.t);

      presentedSheet().pick(0);
      expect(cb.onDisconnect).toHaveBeenCalledTimes(1);
      expect(cb.onReconnect).not.toHaveBeenCalled();
    });

    it('gives selection feedback as the menu opens', () => {
      presentProviderMenu({ providerName: 'Strava', canReconnect: false, ...callbacks() }, i18n.t);
      expect(Haptics.selectionAsync).toHaveBeenCalledTimes(1);
    });

    it('still opens when the haptic engine rejects', () => {
      (Haptics.selectionAsync as jest.Mock).mockRejectedValueOnce(new Error('no engine'));
      presentProviderMenu({ providerName: 'Strava', canReconnect: false, ...callbacks() }, i18n.t);
      expect(presentedSheet().labels).toHaveLength(2);
    });
  });

  describe('on Android', () => {
    beforeEach(() => {
      (Platform as { OS: string }).OS = 'android';
    });

    it('shows the same rows as dialog buttons under the provider name', () => {
      const cb = callbacks();
      presentProviderMenu({ providerName: 'Strava', canReconnect: true, ...cb }, i18n.t);

      const dialog = presentedDialog();
      expect(dialog.title).toBe('Strava');
      expect(dialog.buttons.map((button) => button.text)).toEqual(['Reconnect', 'Disconnect', 'Cancel']);
      expect(dialog.buttons[1].style).toBe('destructive');
      expect(dialog.buttons[2].style).toBe('cancel');
      expect(ActionSheetIOS.showActionSheetWithOptions).not.toHaveBeenCalled();

      dialog.buttons[0].onPress?.();
      expect(cb.onReconnect).toHaveBeenCalledTimes(1);
      dialog.buttons[1].onPress?.();
      expect(cb.onDisconnect).toHaveBeenCalledTimes(1);
    });

    it('drops the reconnect button off a live connection', () => {
      presentProviderMenu({ providerName: 'Garmin', canReconnect: false, ...callbacks() }, i18n.t);

      const dialog = presentedDialog();
      expect(dialog.buttons.map((button) => button.text)).toEqual(['Disconnect', 'Cancel']);
      expect(dialog.buttons[0].style).toBe('destructive');
    });

    it('gives selection feedback as the dialog opens', () => {
      presentProviderMenu({ providerName: 'Strava', canReconnect: false, ...callbacks() }, i18n.t);
      expect(Haptics.selectionAsync).toHaveBeenCalledTimes(1);
    });
  });
});
