// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the remembered-fact menu — forget as the one destructive row, cancel
// ABOUTME: Covers the platform menu on both OSes, the row each index runs, and the haptic on open

import { ActionSheetIOS, Alert, Platform } from 'react-native';
import * as Haptics from 'expo-haptics';
import { i18n } from '@pierre/i18n';

import { presentMemoryFactMenu } from '../src/screens/memory/presentMemoryFactMenu';

type SheetOptions = { options: string[]; cancelButtonIndex?: number; destructiveButtonIndex?: number };
type SheetCallback = (index: number) => void;
type AlertButton = { text?: string; onPress?: () => void; style?: string };

/** The rows the platform sheet last offered, and a way to pick one by index. */
function presentedSheet() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [options, callback] = spy.mock.calls[spy.mock.calls.length - 1] as [SheetOptions, SheetCallback];
  return {
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

describe('the memory fact menu', () => {
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

    it('offers forget as the destructive row, then cancel', () => {
      presentMemoryFactMenu({ onForget: jest.fn() }, i18n.t);

      const sheet = presentedSheet();
      expect(sheet.labels).toEqual(['Forget', 'Cancel']);
      expect(sheet.cancelButtonIndex).toBe(1);
      expect(sheet.destructiveButtonIndex).toBe(0);
    });

    it('forget runs its callback, and cancel runs none', () => {
      const onForget = jest.fn();
      presentMemoryFactMenu({ onForget }, i18n.t);
      const sheet = presentedSheet();

      sheet.pick(0);
      expect(onForget).toHaveBeenCalledTimes(1);
      sheet.pick(1);
      expect(onForget).toHaveBeenCalledTimes(1);
    });

    it('gives selection feedback as the menu opens', () => {
      presentMemoryFactMenu({ onForget: jest.fn() }, i18n.t);
      expect(Haptics.selectionAsync).toHaveBeenCalledTimes(1);
    });

    it('still opens when the haptic engine rejects', () => {
      (Haptics.selectionAsync as jest.Mock).mockRejectedValueOnce(new Error('no engine'));
      presentMemoryFactMenu({ onForget: jest.fn() }, i18n.t);
      expect(presentedSheet().labels).toHaveLength(2);
    });
  });

  describe('on Android', () => {
    beforeEach(() => {
      (Platform as { OS: string }).OS = 'android';
    });

    it('shows forget and cancel as dialog buttons under the confirm title', () => {
      const onForget = jest.fn();
      presentMemoryFactMenu({ onForget }, i18n.t);

      const dialog = presentedDialog();
      expect(dialog.title).toBe('Forget this fact?');
      expect(dialog.buttons.map((button) => button.text)).toEqual(['Forget', 'Cancel']);
      expect(dialog.buttons[0].style).toBe('destructive');
      expect(dialog.buttons[1].style).toBe('cancel');
      expect(ActionSheetIOS.showActionSheetWithOptions).not.toHaveBeenCalled();

      dialog.buttons[0].onPress?.();
      expect(onForget).toHaveBeenCalledTimes(1);
    });

    it('gives selection feedback as the dialog opens', () => {
      presentMemoryFactMenu({ onForget: jest.fn() }, i18n.t);
      expect(Haptics.selectionAsync).toHaveBeenCalledTimes(1);
    });
  });
});
