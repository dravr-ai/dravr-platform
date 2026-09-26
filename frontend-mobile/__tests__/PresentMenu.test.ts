// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the one platform-menu dispatch — the rows it offers, the row each pick runs, the destructive mark
// ABOUTME: Covers the iOS action sheet and the Android dialog, the optional iOS title, and the opt-in haptic

import { ActionSheetIOS, Alert, Platform } from 'react-native';
import * as Haptics from 'expo-haptics';

import { presentMenu, type MenuRow } from '../src/utils/presentMenu';

type SheetOptions = {
  title?: string;
  options: string[];
  cancelButtonIndex?: number;
  destructiveButtonIndex?: number;
};
type SheetCallback = (index: number) => void;
type AlertButton = { text?: string; onPress?: () => void; style?: string };

/** The options the platform sheet last received, and its pick callback. */
function presentedSheet() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalledTimes(1);
  const [options, pick] = spy.mock.calls[0] as [SheetOptions, SheetCallback];
  return { options, pick };
}

/** The dialog Android last showed: its title and its buttons. */
function presentedDialog() {
  const spy = Alert.alert as unknown as jest.Mock;
  expect(spy).toHaveBeenCalledTimes(1);
  const [title, message, buttons] = spy.mock.calls[0] as [string, undefined, AlertButton[]];
  return { title, message, buttons };
}

function threeRows(): { rows: MenuRow[]; handlers: jest.Mock[] } {
  const handlers = [jest.fn(), jest.fn(), jest.fn()];
  const rows = [
    { label: 'Rename', onPress: handlers[0] },
    { label: 'Share', onPress: handlers[1] },
    { label: 'Delete', onPress: handlers[2] },
  ];
  return { rows, handlers };
}

describe('presentMenu', () => {
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

    it('offers the rows then cancel, and marks the destructive index', () => {
      const { rows } = threeRows();
      presentMenu(rows, { title: 'Discussion', cancelLabel: 'Cancel', destructiveIndex: 2 });

      const { options } = presentedSheet();
      expect(options).toEqual({
        title: undefined,
        options: ['Rename', 'Share', 'Delete', 'Cancel'],
        cancelButtonIndex: 3,
        destructiveButtonIndex: 2,
      });
      expect(Alert.alert).not.toHaveBeenCalled();
    });

    it('runs exactly the handler of the row picked, and cancel runs none', () => {
      const { rows, handlers } = threeRows();
      presentMenu(rows, { title: 'Discussion', cancelLabel: 'Cancel' });
      const { pick } = presentedSheet();

      pick(1);
      expect(handlers.map((handler) => handler.mock.calls.length)).toEqual([0, 1, 0]);
      pick(0);
      expect(handlers.map((handler) => handler.mock.calls.length)).toEqual([1, 1, 0]);
      pick(3);
      expect(handlers.map((handler) => handler.mock.calls.length)).toEqual([1, 1, 0]);
    });

    it('carries no destructive row and no title unless asked', () => {
      presentMenu(threeRows().rows, { title: 'Discussion', cancelLabel: 'Cancel' });
      const { options } = presentedSheet();
      expect(options.destructiveButtonIndex).toBeUndefined();
      expect(options.title).toBeUndefined();
    });

    it('heads the sheet with the title when the menu is about one named thing', () => {
      presentMenu(threeRows().rows, { title: 'Strava', cancelLabel: 'Cancel', titleOnIos: true });
      expect(presentedSheet().options.title).toBe('Strava');
    });

    it('gives selection feedback only when asked', () => {
      presentMenu(threeRows().rows, { title: 'Discussion', cancelLabel: 'Cancel' });
      expect(Haptics.selectionAsync).not.toHaveBeenCalled();

      (ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock).mockClear();
      presentMenu(threeRows().rows, { title: 'Discussion', cancelLabel: 'Cancel', haptic: true });
      expect(Haptics.selectionAsync).toHaveBeenCalledTimes(1);
    });

    it('still opens when the haptic engine rejects', () => {
      (Haptics.selectionAsync as jest.Mock).mockRejectedValueOnce(new Error('no engine'));
      presentMenu(threeRows().rows, { title: 'Discussion', cancelLabel: 'Cancel', haptic: true });
      expect(presentedSheet().options.options).toHaveLength(4);
    });
  });

  describe('on Android', () => {
    beforeEach(() => {
      (Platform as { OS: string }).OS = 'android';
    });

    it('shows the rows as dialog buttons under the title, with the destructive one marked', () => {
      const { rows, handlers } = threeRows();
      presentMenu(rows, { title: 'Discussion', cancelLabel: 'Cancel', destructiveIndex: 2 });

      const dialog = presentedDialog();
      expect(dialog.title).toBe('Discussion');
      expect(dialog.message).toBeUndefined();
      expect(dialog.buttons.map((button) => [button.text, button.style])).toEqual([
        ['Rename', 'default'],
        ['Share', 'default'],
        ['Delete', 'destructive'],
        ['Cancel', 'cancel'],
      ]);
      expect(dialog.buttons[3].onPress).toBeUndefined();
      expect(ActionSheetIOS.showActionSheetWithOptions).not.toHaveBeenCalled();

      dialog.buttons[2].onPress?.();
      expect(handlers.map((handler) => handler.mock.calls.length)).toEqual([0, 0, 1]);
    });

    it('marks no button destructive when no index is given', () => {
      presentMenu(threeRows().rows, { title: 'Sort by', cancelLabel: 'Cancel' });
      expect(presentedDialog().buttons.map((button) => button.style)).toEqual([
        'default',
        'default',
        'default',
        'cancel',
      ]);
    });
  });
});
