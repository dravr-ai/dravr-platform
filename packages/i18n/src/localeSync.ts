// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Holds the app-supplied writer that pushes a locale change to users.locale
// ABOUTME: The single seam joining client chrome language to server reply language

import type { SupportedLanguage } from './config';

/**
 * Writes a locale to the signed-in user's account, resolving once the server
 * has it. Each app supplies its own — the web and mobile API clients are
 * separate instances with separate auth storage — and `initI18n` registers it.
 */
export type LocalePersister = (language: SupportedLanguage) => Promise<void>;

let persister: LocalePersister | null = null;

/**
 * Register the writer used by the language-switcher hooks.
 *
 * Called by `initI18n`; exported separately so a test can install a spy
 * without standing up a whole i18next instance.
 */
export function registerLocalePersister(next: LocalePersister): void {
  persister = next;
}

/** Thrown when a locale change is attempted before `initI18n` registered a persister. */
export class LocalePersisterMissingError extends Error {
  constructor() {
    super('No locale persister registered — call initI18n({ persistLocale }) before rendering.');
    this.name = 'LocalePersisterMissingError';
  }
}

/**
 * Push `language` to the server.
 *
 * Rejects rather than swallowing: a language change the server never heard
 * about leaves the coach answering in the old language, and the switcher
 * surfaces that to the user instead of showing a success it cannot vouch for.
 */
export async function persistLocaleToServer(language: SupportedLanguage): Promise<void> {
  if (persister === null) {
    throw new LocalePersisterMissingError();
  }
  await persister(language);
}
