// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that every auth form and the chat composer keep the focused field clear of the keyboard, on Android as well as iOS
// ABOUTME: On a 1080x1600 Android screen the password field once sat entirely behind the keyboard (carnet#353)

import React from 'react';
import { ScrollView, Text, TextInput } from 'react-native';
import { render, screen, within } from '@testing-library/react-native';
import { KeyboardAwareScrollView } from 'react-native-keyboard-controller';

jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'light', setColorScheme: jest.fn() }),
}));

jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => ({ push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
  useLocalSearchParams: () => ({ email: 'jean@example.com' }),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

jest.mock('@expo/vector-icons', () => {
  const { View } = require('react-native');
  return { AntDesign: (props: Record<string, unknown>) => require('react').createElement(View, { testID: `icon-${props.name}` }) };
});

jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ loginWithFirebase: jest.fn(), register: jest.fn() }),
}));

jest.mock('../../../services/api', () => ({
  authApi: { forgotPassword: jest.fn(), resetPassword: jest.fn() },
}));

import { FormScrollView, FORM_KEYBOARD_GAP } from '../FormScrollView';
import { spacing } from '../../../constants/theme';
import { RegisterScreen } from '../../../screens/auth/RegisterScreen';
import { ForgotPasswordScreen } from '../../../screens/auth/ForgotPasswordScreen';
import { ResetPasswordScreen } from '../../../screens/auth/ResetPasswordScreen';

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

/**
 * Every screen that lays out a form the keyboard has to stay clear of. The
 * login screen has no field since the password moved to the server's hosted
 * page (carnet#787), so no keyboard rises over it.
 */
const AUTH_SCREENS: Array<[string, React.ComponentType]> = [
  ['RegisterScreen', RegisterScreen],
  ['ForgotPasswordScreen', ForgotPasswordScreen],
  ['ResetPasswordScreen', ResetPasswordScreen],
];

/**
 * The screens are rendered, not searched for `<FormScrollView`: what matters is
 * that the field the athlete types into sits inside the keyboard-aware
 * container, whichever file the JSX is written in.
 *
 * The library's jest mock renders `KeyboardAwareScrollView` as React Native's
 * own `ScrollView`, so the two cannot be told apart by type here. They are
 * told apart by `bottomOffset`: only the keyboard-aware container takes it,
 * and only `FormScrollView` sets it to the form gap.
 *
 * The root layout's `KeyboardProvider` and the chat column's
 * `KeyboardAvoidingView` were read as text here too; they are rendered in
 * __tests__/ChatLanding.test.tsx and __tests__/ChatScreenNewThreadTitle.test.tsx.
 */
describe.each(AUTH_SCREENS)('%s', (_name, Screen) => {
  it('scrolls every field through FormScrollView, and through nothing else', () => {
    const view = render(<Screen />);

    const scrolls = view.UNSAFE_getAllByType(ScrollView);
    // One container: a second, bare ScrollView around a field is the gap.
    expect(scrolls).toHaveLength(1);
    const form = scrolls[0];
    expect(form.props.bottomOffset).toBe(FORM_KEYBOARD_GAP);
    expect(form.props.keyboardShouldPersistTaps).toBe('handled');

    const fields = view.UNSAFE_getAllByType(TextInput);
    expect(fields.length).toBeGreaterThan(0);
    expect(within(form).UNSAFE_getAllByType(TextInput)).toHaveLength(fields.length);
  });

  it('carries no per-screen copy of the iOS-only inset prop the container replaced', () => {
    const view = render(<Screen />);
    for (const scroll of view.UNSAFE_getAllByType(ScrollView)) {
      expect(scroll.props.automaticallyAdjustKeyboardInsets).toBeUndefined();
    }
  });
});
