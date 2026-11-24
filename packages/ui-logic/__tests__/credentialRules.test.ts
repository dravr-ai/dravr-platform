// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the one password minimum and the one anchored e-mail shape both clients check
// ABOUTME: Red if a seven-character password passes or an eight-character one fails, or the e-mail rule accepts text around an address

import { describe, expect, it } from 'vitest';
import { MIN_PASSWORD_LENGTH, isEmailAddress, isPasswordLongEnough } from '../src/credentialRules';

describe('isPasswordLongEnough', () => {
  // The server (`AuthService::is_valid_password`) accepts eight characters and
  // refuses seven; a form that let seven through would send a password the
  // server then refuses.
  it('refuses seven characters and accepts eight, as the server does', () => {
    expect(isPasswordLongEnough('a'.repeat(7))).toBe(false);
    expect(isPasswordLongEnough('a'.repeat(8))).toBe(true);
    expect(isPasswordLongEnough('')).toBe(false);
  });

  it('starts accepting at the length the forms print in their hint', () => {
    expect(isPasswordLongEnough('a'.repeat(MIN_PASSWORD_LENGTH - 1))).toBe(false);
    expect(isPasswordLongEnough('a'.repeat(MIN_PASSWORD_LENGTH))).toBe(true);
  });
});

describe('isEmailAddress', () => {
  it('accepts a plain address', () => {
    expect(isEmailAddress('athlete@example.com')).toBe(true);
    expect(isEmailAddress('first.last+tag@mail.example.org')).toBe(true);
  });

  it('refuses a value with no @, no domain dot, or nothing on one side', () => {
    expect(isEmailAddress('')).toBe(false);
    expect(isEmailAddress('athlete')).toBe(false);
    expect(isEmailAddress('athlete@example')).toBe(false);
    expect(isEmailAddress('@example.com')).toBe(false);
    expect(isEmailAddress('athlete@')).toBe(false);
  });

  it('is anchored: text around an address is not an address', () => {
    // The unanchored `/\S+@\S+\.\S+/` the phone used accepted all three.
    expect(isEmailAddress('write to athlete@example.com')).toBe(false);
    expect(isEmailAddress('athlete@example.com today')).toBe(false);
    expect(isEmailAddress(' athlete@example.com')).toBe(false);
    expect(isEmailAddress('a@b@example.com')).toBe(false);
  });
});
