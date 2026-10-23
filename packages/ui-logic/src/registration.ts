// ABOUTME: The sentence a new account is greeted with, in the athlete's language, from what it waits on
// ABOUTME: Shared by web and mobile so both say "confirm your address" and "awaiting review" the same way
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { RegisterResponse } from '@pierre/shared-types';
import type { ApiErrorTranslate } from './apiError';

/**
 * What a registration tells the athlete, worded in their language.
 *
 * The server's `message` is English only; `activation` says the same thing in
 * a form each client can translate.
 */
export function describeRegistration(
  response: Pick<RegisterResponse, 'activation'>,
  t: ApiErrorTranslate,
): string {
  switch (response.activation) {
    case 'active':
      return t('auth.registeredReady');
    case 'confirm_email':
      return t('auth.registeredConfirmEmail');
    case 'review':
      return t('auth.registeredReview');
  }
}
