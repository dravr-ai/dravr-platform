// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the message long-press menu — copy, share, rate, retry on the last agent turn, cancel
// ABOUTME: Covers the platform menu on both OSes, the row each index runs, the held rating's mark, and the haptic on open

import { ActionSheetIOS, Alert, Platform } from 'react-native';
import * as Haptics from 'expo-haptics';
import { i18n } from '@pierre/i18n';

import { presentMessageMenu } from '../src/screens/chat/presentMessageMenu';

type SheetOptions = { options: string[]; cancelButtonIndex?: number };
type SheetCallback = (index: number) => void;
type AlertButton = { text?: string; onPress?: () => void; style?: string };

/** The rows the platform sheet last offered, and a way to pick one by index. */
function presentedSheet() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [options, callback] = spy.mock.calls[spy.mock.calls.length - 1] as [SheetOptions, SheetCallback];
  return { labels: options.options, cancelButtonIndex: options.cancelButtonIndex, pick: callback };
}

/** The dialog Android last showed: its title and its buttons. */
function presentedDialog() {
  const spy = Alert.alert as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [title, , buttons] = spy.mock.calls[spy.mock.calls.length - 1] as [string, undefined, AlertButton[]];
  return { title, buttons };
}

function callbacks() {
  return { onCopy: jest.fn(), onShare: jest.fn(), onRate: jest.fn(), onRetry: jest.fn() };
}

describe('the message menu', () => {
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

    it('offers copy, share, both ratings, retry and cancel on the last agent turn', () => {
      presentMessageMenu({ canRetry: true, rating: null, ...callbacks() }, i18n.t);

      const sheet = presentedSheet();
      expect(sheet.labels).toEqual(['Copy', 'Share', 'Good response', 'Poor response', 'Retry', 'Cancel']);
      expect(sheet.cancelButtonIndex).toBe(5);
    });

    it('offers no retry on any other message', () => {
      presentMessageMenu({ canRetry: false, rating: null, ...callbacks() }, i18n.t);

      const sheet = presentedSheet();
      expect(sheet.labels).toEqual(['Copy', 'Share', 'Good response', 'Poor response', 'Cancel']);
      expect(sheet.cancelButtonIndex).toBe(4);
    });

    it('each row runs its own callback, and cancel runs none', () => {
      const cb = callbacks();
      presentMessageMenu({ canRetry: true, rating: null, ...cb }, i18n.t);
      const sheet = presentedSheet();

      sheet.pick(0);
      expect(cb.onCopy).toHaveBeenCalledTimes(1);
      sheet.pick(1);
      expect(cb.onShare).toHaveBeenCalledTimes(1);
      sheet.pick(2);
      expect(cb.onRate).toHaveBeenLastCalledWith('up');
      sheet.pick(3);
      expect(cb.onRate).toHaveBeenLastCalledWith('down');
      expect(cb.onRate).toHaveBeenCalledTimes(2);
      sheet.pick(4);
      expect(cb.onRetry).toHaveBeenCalledTimes(1);

      sheet.pick(5);
      expect(cb.onCopy).toHaveBeenCalledTimes(1);
      expect(cb.onShare).toHaveBeenCalledTimes(1);
      expect(cb.onRate).toHaveBeenCalledTimes(2);
      expect(cb.onRetry).toHaveBeenCalledTimes(1);
    });

    // The sheet cannot draw a checkmark of its own, so the held rating's row
    // carries one in its label; tapping it still reports that rating, and the
    // caller is the one that toggles it off.
    it('marks the held rating and still reports it when tapped', () => {
      const cb = callbacks();
      presentMessageMenu({ canRetry: false, rating: 'down', ...cb }, i18n.t);

      const sheet = presentedSheet();
      expect(sheet.labels).toEqual(['Copy', 'Share', 'Good response', '✓ Poor response', 'Cancel']);
      sheet.pick(3);
      expect(cb.onRate).toHaveBeenCalledWith('down');
    });

    it('marks a held thumbs-up the same way', () => {
      presentMessageMenu({ canRetry: false, rating: 'up', ...callbacks() }, i18n.t);
      expect(presentedSheet().labels[2]).toBe('✓ Good response');
    });

    it('gives selection feedback as the menu opens', () => {
      presentMessageMenu({ canRetry: false, rating: null, ...callbacks() }, i18n.t);
      expect(Haptics.selectionAsync).toHaveBeenCalledTimes(1);
    });

    it('still opens when the haptic engine rejects', () => {
      (Haptics.selectionAsync as jest.Mock).mockRejectedValueOnce(new Error('no engine'));
      presentMessageMenu({ canRetry: false, rating: null, ...callbacks() }, i18n.t);
      expect(presentedSheet().labels).toHaveLength(5);
    });
  });

  describe('on Android', () => {
    beforeEach(() => {
      (Platform as { OS: string }).OS = 'android';
    });

    it('shows the same rows as dialog buttons under the message-actions title', () => {
      const cb = callbacks();
      presentMessageMenu({ canRetry: true, rating: null, ...cb }, i18n.t);

      const dialog = presentedDialog();
      expect(dialog.title).toBe('Message actions');
      expect(dialog.buttons.map((button) => button.text)).toEqual([
        'Copy',
        'Share',
        'Good response',
        'Poor response',
        'Retry',
        'Cancel',
      ]);
      expect(dialog.buttons[5].style).toBe('cancel');
      expect(ActionSheetIOS.showActionSheetWithOptions).not.toHaveBeenCalled();

      dialog.buttons[0].onPress?.();
      expect(cb.onCopy).toHaveBeenCalledTimes(1);
      dialog.buttons[3].onPress?.();
      expect(cb.onRate).toHaveBeenCalledWith('down');
      dialog.buttons[4].onPress?.();
      expect(cb.onRetry).toHaveBeenCalledTimes(1);
    });

    it('drops the retry button off any other message', () => {
      presentMessageMenu({ canRetry: false, rating: null, ...callbacks() }, i18n.t);
      expect(presentedDialog().buttons.map((button) => button.text)).toEqual([
        'Copy',
        'Share',
        'Good response',
        'Poor response',
        'Cancel',
      ]);
    });

    it('gives selection feedback as the dialog opens', () => {
      presentMessageMenu({ canRetry: false, rating: null, ...callbacks() }, i18n.t);
      expect(Haptics.selectionAsync).toHaveBeenCalledTimes(1);
    });
  });
});
