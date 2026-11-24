// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: How the day editor reads a number field the coach types into — minutes, metres, a percent, an RPE (carnet#1714, #1727)
// ABOUTME: Digits only; an emptied field reads as no figure at all, so the coach can clear it to type a new one

/** A field's value as digits, or `undefined` once it is emptied. */
export function digitsOf(text: string): number | undefined {
  const digits = text.replace(/\D/g, '');
  return digits === '' ? undefined : Number(digits);
}
