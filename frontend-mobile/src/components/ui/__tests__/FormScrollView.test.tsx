// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that every auth form and the chat composer keep the focused field clear of the keyboard, on Android as well as iOS
// ABOUTME: On a 1080x1600 Android screen the password field once sat entirely behind the keyboard (carnet#353)

import React from 'react';
import { Text } from 'react-native';
import { render, screen } from '@testing-library/react-native';
import { KeyboardAwareScrollView } from 'react-native-keyboard-controller';
import { readFileSync } from 'fs';
import { join } from 'path';
import { FormScrollView, FORM_KEYBOARD_GAP } from '../FormScrollView';
import { spacing } from '../../../constants/theme';

describe('FormScrollView', () => {
  /**
   * carnet#353: `automaticallyAdjustKeyboardInsets` is iOS-only and React
   * Native's `KeyboardAvoidingView` is inert under Expo's mandatory Android
   * edge-to-edge, so the container is the keyboard-controller scroll view,
   * which reads the IME inset natively on both platforms.
   */
  it('is the keyboard-aware scroll view, with a gap that keeps the error line readable', () => {
    render(
      <FormScrollView testID="form">
        <Text>field</Text>
      </FormScrollView>,
    );

    const form = screen.UNSAFE_getByType(KeyboardAwareScrollView);
    expect(form.props.testID).toBe('form');
    expect(form.props.bottomOffset).toBe(FORM_KEYBOARD_GAP);
    expect(FORM_KEYBOARD_GAP).toBe(spacing.md);
    expect(FORM_KEYBOARD_GAP).toBeGreaterThan(0);
  });

  it('lets a tap on a control land while the keyboard is up', () => {
    render(
      <FormScrollView testID="form">
        <Text>field</Text>
      </FormScrollView>,
    );

    expect(screen.getByTestId('form').props.keyboardShouldPersistTaps).toBe('handled');
  });

  it('keeps the caller style and forwards the other props the caller set', () => {
    render(
      <FormScrollView
        testID="form"
        contentContainerStyle={{ flexGrow: 1, paddingHorizontal: 24 }}
        showsVerticalScrollIndicator={false}
      >
        <Text>field</Text>
      </FormScrollView>,
    );

    const form = screen.getByTestId('form');
    const style = form.props.contentContainerStyle as Record<string, number>;
    expect(style.flexGrow).toBe(1);
    expect(style.paddingHorizontal).toBe(24);
    expect(form.props.showsVerticalScrollIndicator).toBe(false);
  });
});

const MOBILE_ROOT = join(__dirname, '..', '..', '..', '..');
const read = (relative: string): string => readFileSync(join(MOBILE_ROOT, relative), 'utf8');

/** Every screen that lays out a form the keyboard has to stay clear of. */
const AUTH_SCREENS = [
  'src/screens/auth/LoginScreen.tsx',
  'src/screens/auth/RegisterScreen.tsx',
  'src/screens/auth/ForgotPasswordScreen.tsx',
  'src/screens/auth/ResetPasswordScreen.tsx',
];

describe.each(AUTH_SCREENS)('%s', (file) => {
  const src = read(file);

  it('reads a real screen, so an empty read cannot pass as compliance', () => {
    expect(src.length).toBeGreaterThan(500);
    expect(src).toContain('<Input');
  });

  it('scrolls its form through FormScrollView, not a bare ScrollView', () => {
    expect(src).toContain('<FormScrollView');
    expect(src).toContain('</FormScrollView>');
    expect(src).not.toContain('<ScrollView');
  });

  it('carries no per-screen copy of the iOS-only inset prop the container replaced', () => {
    expect(src).not.toContain('automaticallyAdjustKeyboardInsets');
  });
});

describe('the root layout', () => {
  it('mounts the keyboard tracker every FormScrollView reads', () => {
    const layout = read('app/_layout.tsx');
    expect(layout).toContain("import { KeyboardProvider } from 'react-native-keyboard-controller';");
    // Above the navigators: the provider must enclose <RootShell />, which is
    // where every route mounts.
    const provider = layout.indexOf('<KeyboardProvider>');
    const shell = layout.indexOf('<RootShell />');
    const close = layout.indexOf('</KeyboardProvider>');
    expect(provider).toBeGreaterThan(-1);
    expect(shell).toBeGreaterThan(provider);
    expect(close).toBeGreaterThan(shell);
  });
});

describe('the chat screen', () => {
  const src = read('src/screens/chat/ChatScreen.tsx');

  it('pads its column for the keyboard through the same tracker, on both platforms', () => {
    // React Native's own KeyboardAvoidingView is the one that measured
    // nothing under Android edge-to-edge, so the composer sat behind the
    // keyboard; the keyboard-controller one reads the IME inset itself.
    expect(src).toContain("import { KeyboardAvoidingView } from 'react-native-keyboard-controller';");
    expect(src).toMatch(/<KeyboardAvoidingView\s+style=\{\{ flex: 1 \}\}\s+behavior="padding"\s+keyboardVerticalOffset=\{headerHeight\}/);
    // No platform gate: Android was the platform left without padding.
    expect(src).not.toMatch(/behavior=\{Platform\.OS/);
    expect(src).not.toMatch(/keyboardVerticalOffset=\{Platform\.OS/);
  });
});
