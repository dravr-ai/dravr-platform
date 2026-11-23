// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's "Who read your data" card: each credential names its own label, a Connected app none
// ABOUTME: Every key the card asks for resolves in all five catalogues, so no client prints a raw key (carnet#1543)

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { countKey } from '@pierre/chat-utils';
import {
  ACCESS_FEED_ALL_KEYS,
  ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS,
  ACCESS_FEED_DAY_READS_KEYS,
  ACCESS_FEED_DAY_VIEWS_KEYS,
  ACCESS_FEED_PEER_DAY_READS_KEYS,
  ACCESS_FEED_PEER_DAY_VIEWS_KEYS,
  accessFeedCredentialKey,
} from '../src/access-feed';

const LOCALES = ['en', 'fr', 'es', 'de', 'pt'] as const;

/** The catalogue a locale's clients read: the bump lane's copy of dravr-contremaitre `strings/`. */
function catalogue(locale: string): unknown {
  const path = resolve(__dirname, '../../i18n/src/locales', locale, 'translation.json');
  return JSON.parse(readFileSync(path, 'utf8'));
}

/** The value at a dotted key of a nested catalogue, or `undefined`. */
function lookup(tree: unknown, key: string): unknown {
  return key
    .split('.')
    .reduce<unknown>(
      (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
      tree,
    );
}

describe('accessFeedCredentialKey', () => {
  it('labels every credential that is not a Connected app, each with its own words', () => {
    const keys = (['api_key', 'mcp_token', 'first_party_session', 'a2a_client_credentials'] as const).map(
      accessFeedCredentialKey,
    );
    expect(keys.every((key) => key?.startsWith('accessFeed.credential.'))).toBe(true);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('labels neither a Connected app, whose Revoke says what it is, nor an unrecorded credential', () => {
    expect(accessFeedCredentialKey('oauth_grant')).toBeNull();
    expect(accessFeedCredentialKey(null)).toBeNull();
  });
});

describe('the day counts', () => {
  it('read the singular for one, and for a French zero', () => {
    expect(countKey(1, 'en', ACCESS_FEED_DAY_READS_KEYS)).toBe('humanCoach.readLog.dayReads_one');
    expect(countKey(3, 'en', ACCESS_FEED_DAY_VIEWS_KEYS)).toBe('humanCoach.readLog.dayViews_other');
    expect(countKey(0, 'fr', ACCESS_FEED_DAY_READS_KEYS)).toBe('humanCoach.readLog.dayReads_one');
    expect(countKey(1, 'en', ACCESS_FEED_PEER_DAY_READS_KEYS)).toBe('accessFeed.peerDayReads_one');
    expect(countKey(2, 'en', ACCESS_FEED_PEER_DAY_READS_KEYS)).toBe('accessFeed.peerDayReads_other');
  });
});

describe('the catalogue', () => {
  it.each(LOCALES)('carries every key the card asks for in %s', (locale) => {
    // The keys are authored in dravr-contremaitre and copied here by the bump
    // lane, so a key this module names that the copy lacks prints raw on both
    // clients.
    const tree = catalogue(locale);
    const missing = ACCESS_FEED_ALL_KEYS.filter((key) => typeof lookup(tree, key) !== 'string');
    expect(missing).toEqual([]);
  });

  it.each(LOCALES)('gives each day count its {{count}} and {{group}} slots in %s', (locale) => {
    const tree = catalogue(locale);
    for (const pair of [ACCESS_FEED_DAY_READS_KEYS, ACCESS_FEED_DAY_VIEWS_KEYS, ACCESS_FEED_PEER_DAY_VIEWS_KEYS]) {
      for (const key of [pair.one, pair.many]) {
        const words = lookup(tree, key);
        expect(words, `${locale}:${key}`).toEqual(expect.stringContaining('{{count}}'));
        expect(words, `${locale}:${key}`).toEqual(expect.stringContaining('{{group}}'));
      }
    }
  });

  it.each(LOCALES)('gives a peer day its {{member}} and {{group}} slots, and {{count}} past one, in %s', (locale) => {
    const tree = catalogue(locale);
    for (const key of [ACCESS_FEED_PEER_DAY_READS_KEYS.one, ACCESS_FEED_PEER_DAY_READS_KEYS.many]) {
      const words = lookup(tree, key);
      expect(words, `${locale}:${key}`).toEqual(expect.stringContaining('{{member}}'));
      expect(words, `${locale}:${key}`).toEqual(expect.stringContaining('{{group}}'));
    }
    expect(lookup(tree, ACCESS_FEED_PEER_DAY_READS_KEYS.many)).toEqual(expect.stringContaining('{{count}}'));
  });

  it.each(LOCALES)('gives an API key\'s day count its {{count}} slot in %s', (locale) => {
    const tree = catalogue(locale);
    for (const key of [ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.one, ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.many]) {
      expect(lookup(tree, key), `${locale}:${key}`).toEqual(expect.stringContaining('{{count}}'));
    }
  });
});
