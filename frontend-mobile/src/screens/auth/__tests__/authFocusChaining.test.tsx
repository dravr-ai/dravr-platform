// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the auth forms chain focus field-to-field, and that Input forwards the ref that makes it possible
// ABOUTME: returnKeyType only relabels the return key — without a ref nothing moves the caret, which is the carnet#353 gap

import React, { createRef } from 'react';
import { Alert, TextInput } from 'react-native';
import { fireEvent, render, waitFor, type RenderResult } from '@testing-library/react-native';

jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'light', setColorScheme: jest.fn() }),
}));

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({ email: 'jean@example.com' }),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

jest.mock('@expo/vector-icons', () => {
  const { View } = require('react-native');
  return { AntDesign: (props: Record<string, unknown>) => require('react').createElement(View, { testID: `icon-${props.name}` }) };
});

const mockRegister = jest.fn();
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({
    loginWithFirebase: jest.fn(),
    register: (...args: unknown[]) => mockRegister(...args),
  }),
}));

const mockForgotPassword = jest.fn();
const mockResetPassword = jest.fn();
jest.mock('../../../services/api', () => ({
  authApi: {
    forgotPassword: (...args: unknown[]) => mockForgotPassword(...args),
    resetPassword: (...args: unknown[]) => mockResetPassword(...args),
  },
}));

import { Input } from '../../../components/ui/Input';
import { RegisterScreen } from '../RegisterScreen';
import { ForgotPasswordScreen } from '../ForgotPasswordScreen';
import { ResetPasswordScreen } from '../ResetPasswordScreen';

describe('Input ref forwarding', () => {
  it('hands back the underlying TextInput, so a screen can focus it', () => {
    const ref = createRef<TextInput>();
    render(<Input ref={ref} testID="probe" />);
    // A plain function component silently drops the ref and leaves this null —
    // which is exactly how the auth screens ended up unable to chain focus.
    expect(ref.current).not.toBeNull();
    expect(typeof ref.current?.focus).toBe('function');
  });
});

const RESET_CODE = 'abcd1234EFGH5678.abcd1234EFGH5678ijkl9012MNOP3456';

/**
 * Each form as the athlete fills it: the fields in order with a valid value,
 * and the call the form makes once the last field's return key submits it.
 * Every field but the last hands off to the next; the last submits.
 */
const FORMS: Array<{
  name: string;
  Screen: React.ComponentType;
  fields: Array<[testID: string, value: string]>;
  submit: jest.Mock;
  submittedWith: unknown[];
}> = [
  {
    name: 'RegisterScreen',
    Screen: RegisterScreen,
    fields: [
      ['register-display-name-input', 'Jean'],
      ['register-email-input', 'jean@example.com'],
      ['register-password-input', 'ValidPassword123'],
      ['register-confirm-password-input', 'ValidPassword123'],
    ],
    submit: mockRegister,
    submittedWith: ['jean@example.com', 'ValidPassword123', 'Jean'],
  },
  {
    name: 'ResetPasswordScreen',
    Screen: ResetPasswordScreen,
    fields: [
      ['reset-code-input', RESET_CODE],
      ['new-password-input', 'ValidPassword123'],
      ['confirm-password-input', 'ValidPassword123'],
    ],
    submit: mockResetPassword,
    submittedWith: [RESET_CODE, 'ValidPassword123'],
  },
  {
    // A single field, so it submits with no chain.
    name: 'ForgotPasswordScreen',
    Screen: ForgotPasswordScreen,
    fields: [['forgot-email-input', 'jean@example.com']],
    submit: mockForgotPassword,
    submittedWith: ['jean@example.com'],
  },
];

/**
 * The fields `focus()` was called on, in order, by test id.
 *
 * Under jest a `TextInput` is a class whose `focus` is one mock on the
 * prototype, so each call's `this` is the input that was asked to take the
 * caret — the field a screen's ref points at, not a name in its source.
 */
function focusedFields(): string[] {
  const focus = TextInput.prototype.focus as unknown as jest.Mock;
  return focus.mock.contexts.map((input) => (input as { props: { testID?: string } }).props.testID ?? '?');
}

/** Press the return key on a field, as the keyboard does. */
function pressReturn(view: RenderResult, testID: string): void {
  fireEvent(view.getByTestId(testID), 'submitEditing');
}

/**
 * Each form is rendered and its return keys are pressed. Matching
 * `returnKeyType="next"`, `onSubmitEditing={() => x.current?.focus()}` and
 * `ref={x}` in a screen's source says nothing about WHICH field a ref lands
 * on: an email field wired to focus itself passes it.
 */
describe.each(FORMS)('$name focus chain', ({ Screen, fields, submit, submittedWith }) => {
  const ids = fields.map(([testID]) => testID);
  const handoffs = ids.slice(0, -1).map((id, index) => [id, ids[index + 1]]);
  const last = ids[ids.length - 1];

  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => {});
    submit.mockResolvedValue(undefined);
  });

  if (handoffs.length > 0) {
    it.each(handoffs)('%s hands the caret to %s', (from, to) => {
      const view = render(<Screen />);
      const field = view.getByTestId(from);

      expect(field.props.returnKeyType).toBe('next');
      // Without this the keyboard closes between fields and the handoff flickers.
      expect(field.props.blurOnSubmit).toBe(false);

      pressReturn(view, from);

      expect(focusedFields()).toEqual([to]);
      // Moving on is not submitting: nothing was sent, and nothing was refused.
      expect(submit).not.toHaveBeenCalled();
    });
  }

  it('the last field submits rather than handing off', async () => {
    const view = render(<Screen />);
    for (const [testID, value] of fields) fireEvent.changeText(view.getByTestId(testID), value);

    expect(view.getByTestId(last).props.returnKeyType).toBe('go');
    pressReturn(view, last);

    await waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
    expect(submit).toHaveBeenCalledWith(...submittedWith);
    expect(focusedFields()).toEqual([]);
  });
});
