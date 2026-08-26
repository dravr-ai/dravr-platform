// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile app's writer from a chosen chrome language to users.locale
// ABOUTME: Registered once by the root layout so every language change reaches the server

import type { LocalePersister } from '@pierre/i18n';
import { userApi } from '../services/api';

/**
 * Push the chosen language to `users.locale` via `PUT /api/user/locale`.
 *
 * Rejects on failure so the language switcher can tell the user the chrome
 * moved but the coach's reply language did not.
 */
export const persistLocale: LocalePersister = async (language) => {
  await userApi.updateLocale(language);
};
