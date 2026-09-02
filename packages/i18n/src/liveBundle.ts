// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Overlays the live string catalogue from the server on the resources embedded at build time
// ABOUTME: Nests the flat map before adding it, because i18next files a dotted key as a literal otherwise

import type { i18n as I18nInstance } from 'i18next';
import type { I18nBundle, I18nBundleResult } from '@pierre/shared-types';

/** Reads the live catalogue for one locale; `etag` is the digest of the bundle already held. */
export type BundleFetcher = (locale: string, etag?: string) => Promise<I18nBundleResult>;

/** The installed overlay: refresh one locale on demand, or stop listening. */
export interface LiveOverlay {
  /** Fetch `locale` and apply it if it changed. Resolves either way; a failed fetch changes nothing. */
  refresh: (locale: string) => Promise<void>;
  /** Stop refreshing on language change. */
  dispose: () => void;
}

/** The namespace every catalogue key lives in. */
const NAMESPACE = 'translation';

/**
 * Turn `{ "a.b": "x" }` into `{ a: { b: "x" } }`.
 *
 * `addResourceBundle` merges the object it is handed as-is: a flat dotted key
 * lands as a literal top-level key that `t('a.b')` never resolves, so the
 * overlay would apply without error and change nothing on screen. Throws on a
 * key that nests under a leaf or lands on a subtree — the catalogue never
 * ships either, and silently keeping one would hide a broken key.
 */
export function nestDotted(flat: Record<string, string>): Record<string, unknown> {
  const root: Record<string, unknown> = {};
  for (const [dotted, value] of Object.entries(flat)) {
    const parts = dotted.split('.');
    let node = root;
    for (const part of parts.slice(0, -1)) {
      const next = node[part];
      if (next === undefined) {
        const child: Record<string, unknown> = {};
        node[part] = child;
        node = child;
      } else if (typeof next === 'object' && next !== null) {
        node = next as Record<string, unknown>;
      } else {
        throw new Error(`catalogue key ${dotted} nests under the leaf ${part}`);
      }
    }
    const leaf = parts[parts.length - 1];
    if (typeof node[leaf] === 'object' && node[leaf] !== null) {
      throw new Error(`catalogue key ${dotted} is already a subtree`);
    }
    node[leaf] = value;
  }
  return root;
}

/**
 * Merge a live bundle over the locale's embedded resources.
 *
 * Deep merge, overwrite: a key the server has wins, a key it lacks keeps its
 * embedded text. Mounted components repaint because the config binds them to
 * the store's `added` event (`bindI18nStore` in `defaultI18nConfig`).
 */
export function applyLiveBundle(i18n: I18nInstance, bundle: I18nBundle): void {
  i18n.addResourceBundle(bundle.locale, NAMESPACE, nestDotted(bundle.strings), true, true);
}

/**
 * Overlay the live catalogue now and again on every language change.
 *
 * Fire-and-forget by design: the embedded copy is always a correct catalogue,
 * so a failed or slow fetch must never delay first paint or surface as an
 * error. The digest of each bundle applied is sent back on the next fetch, so
 * an unchanged catalogue costs a bodiless 304.
 */
export function installLiveOverlay(i18n: I18nInstance, fetchBundle: BundleFetcher): LiveOverlay {
  const etags = new Map<string, string>();
  const refresh = async (locale: string): Promise<void> => {
    try {
      const result = await fetchBundle(locale, etags.get(locale));
      if (result.status === 'fresh') {
        etags.set(result.bundle.locale, result.bundle.etag);
        applyLiveBundle(i18n, result.bundle);
      }
    } catch {
      // The embedded catalogue stays on screen; the next open tries again.
    }
  };
  const onLanguageChanged = (locale: string): void => {
    void refresh(locale);
  };
  i18n.on('languageChanged', onLanguageChanged);
  void refresh(i18n.language);
  return {
    refresh,
    dispose: () => {
      i18n.off('languageChanged', onLanguageChanged);
    },
  };
}
