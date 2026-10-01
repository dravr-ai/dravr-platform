// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The catalogue key naming each account role, read by the Account pane on both clients and the web sidebar
// ABOUTME: One table, so no surface prints the raw enum (`user`, `super_admin`) under a translated label

import type { UserRole } from '@pierre/shared-types';

/**
 * The corpus key naming each account role. The Account pane on web and mobile
 * and the web sidebar's operator badge read this one table; the pane used to
 * print the wire value capitalised, which read "User" under French chrome.
 */
export const ACCOUNT_ROLE_LABEL_KEY: Record<UserRole, string> = {
  super_admin: 'shell.roleSuperAdmin',
  admin: 'shell.roleAdmin',
  user: 'shell.roleUser',
};
