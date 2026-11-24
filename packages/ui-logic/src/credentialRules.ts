// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The password and e-mail rules every sign-up, reset and account form checks before it sends
// ABOUTME: One minimum length and one e-mail shape for both clients, matching what the server accepts

/**
 * The shortest password the server accepts (`AuthService::is_valid_password`).
 * Every form refuses a shorter one before sending, and prints this number in
 * its hint.
 */
export const MIN_PASSWORD_LENGTH = 8;

/** True when `password` is long enough for the server to accept it. */
export function isPasswordLongEnough(password: string): boolean {
  return password.length >= MIN_PASSWORD_LENGTH;
}

/** Anchored: the whole value is one address, with nothing before or after it. */
const EMAIL_ADDRESS = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/**
 * True when `value` is shaped like one e-mail address: a local part, one `@`,
 * and a dotted domain, with no whitespace. A caller that accepts surrounding
 * spaces trims before asking.
 */
export function isEmailAddress(value: string): boolean {
  return EMAIL_ADDRESS.test(value);
}
