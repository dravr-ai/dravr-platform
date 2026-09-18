// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the auth forms chain focus field-to-field, and that Input forwards the ref that makes it possible
// ABOUTME: returnKeyType only relabels the return key — without a ref nothing moves the caret, which is the carnet#353 gap

import React, { createRef } from 'react';
import { readFileSync } from 'fs';
import { join } from 'path';
import { render } from '@testing-library/react-native';
import type { TextInput } from 'react-native';

jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'light', setColorScheme: jest.fn() }),
}));

import { Input } from '../../../components/ui/Input';

const AUTH_DIR = join(__dirname, '..');
const read = (f: string): string => readFileSync(join(AUTH_DIR, f), 'utf8');

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

/**
 * The chain each form must implement, as (file, ordered testIDs). Every field
 * but the last hands off to the next; the last submits.
 */
const CHAINS: Array<[string, string[]]> = [
  ['LoginScreen.tsx', ['email-input', 'password-input']],
  [
    'RegisterScreen.tsx',
    [
      'register-display-name-input',
      'register-email-input',
      'register-password-input',
      'register-confirm-password-input',
    ],
  ],
  ['ResetPasswordScreen.tsx', ['reset-code-input', 'new-password-input', 'confirm-password-input']],
];

describe.each(CHAINS)('%s focus chain', (file, ids) => {
  const src = read(file);

  it('reads a real screen, so an empty read cannot pass as compliance', () => {
    expect(src.length).toBeGreaterThan(500);
    for (const id of ids) expect(src).toContain(`testID="${id}"`);
  });

  /** The props block of the `<Input>` carrying this testID. */
  const blockFor = (id: string): string => {
    const at = src.indexOf(`testID="${id}"`);
    const open = src.lastIndexOf('<Input', at);
    return src.slice(open, at);
  };

  it.each(ids.slice(0, -1))('%s hands off to the next field', (id) => {
    const block = blockFor(id);
    expect(block).toContain('returnKeyType="next"');
    expect(block).toMatch(/onSubmitEditing=\{\(\) => \w+\.current\?\.focus\(\)\}/);
    // Without this the keyboard closes between fields and the handoff flickers.
    expect(block).toContain('blurOnSubmit={false}');
  });

  it.each(ids.slice(1))('%s is reachable — it takes a ref', (id) => {
    expect(blockFor(id)).toMatch(/ref=\{\w+\}/);
  });

  it('the last field submits rather than handing off', () => {
    const block = blockFor(ids[ids.length - 1]);
    expect(block).toContain('returnKeyType="go"');
    expect(block).toMatch(/onSubmitEditing=\{handle\w+\}/);
  });
});

describe('ForgotPasswordScreen', () => {
  it('has a single field, so it submits with no chain', () => {
    const src = read('ForgotPasswordScreen.tsx');
    expect(src).toContain('testID="forgot-email-input"');
    expect(src).toContain('returnKeyType="go"');
    expect(src).toMatch(/onSubmitEditing=\{handle\w+\}/);
  });
});
