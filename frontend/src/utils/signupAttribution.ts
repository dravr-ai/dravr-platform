// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Keeps where a visitor came from (utm_* campaign, dravr.ai button) until they sign up, for user.signed_up
// ABOUTME: Read from the query before or after the hash on page load, held in sessionStorage, cleared once the account exists

import type { SignupAttribution } from '@pierre/shared-types';

/** The sessionStorage key the landing attribution survives the session's navigation under. */
export const SIGNUP_ATTRIBUTION_KEY = 'pierre_signup_attribution';

/**
 * The link parameters a sign-up carries: the `utm_*` campaign tags, and the
 * `ref`, `placement` and `lang` a dravr.ai create-account button adds.
 */
const ATTRIBUTION_KEYS = [
  'utm_source',
  'utm_medium',
  'utm_campaign',
  'utm_term',
  'utm_content',
  'ref',
  'placement',
  'lang',
] as const;

/**
 * Longest value kept. The server is the judge of what a campaign tag is (it
 * records only short tag-shaped values and drops the rest); this bound only
 * keeps a pasted payload out of storage and out of the request. A longer value
 * is dropped, never cut: a truncated tag would name a campaign nobody ran.
 */
const MAX_VALUE_LENGTH = 200;

/**
 * The attribution parameters of an address: the query before the hash
 * (`/?utm_source=x#home`) and the one after it (`/#/signup?utm_source=x`),
 * the first usable value of each winning. Null when there are none.
 */
export function readAttributionParameters(location: Pick<Location, 'search' | 'hash'>): SignupAttribution | null {
  const hashQuery = location.hash.indexOf('?');
  const sources = [
    new URLSearchParams(location.search),
    new URLSearchParams(hashQuery === -1 ? '' : location.hash.slice(hashQuery + 1)),
  ];
  const attribution: SignupAttribution = {};
  for (const key of ATTRIBUTION_KEYS) {
    for (const params of sources) {
      const value = params.get(key)?.trim();
      if (value && value.length <= MAX_VALUE_LENGTH) {
        attribution[key] = value;
        break;
      }
    }
  }
  return Object.keys(attribution).length > 0 ? attribution : null;
}

/**
 * Keep the attribution this page was opened with, when its address names one.
 * A newer link (a campaign or a dravr.ai button) replaces the whole record; an
 * address with none leaves the kept one alone, since the app rewrites its own
 * address as it routes. Call it once, as the app loads, before anything
 * rewrites the address.
 */
export function captureSignupAttribution(location: Pick<Location, 'search' | 'hash'> = window.location): void {
  const attribution = readAttributionParameters(location);
  if (attribution === null) return;
  try {
    sessionStorage.setItem(SIGNUP_ATTRIBUTION_KEY, JSON.stringify(attribution));
  } catch {
    // Storage blocked (private mode, site data off): the sign-up simply
    // carries no campaign.
  }
}

function isSignupAttribution(value: unknown): value is SignupAttribution {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as Record<string, unknown>;
  return ATTRIBUTION_KEYS.every((key) => v[key] === undefined || typeof v[key] === 'string');
}

/** The attribution kept for this session's sign-up, or null when there is none. */
export function readSignupAttribution(): SignupAttribution | null {
  try {
    const raw = sessionStorage.getItem(SIGNUP_ATTRIBUTION_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!isSignupAttribution(parsed)) return null;
    const attribution: SignupAttribution = {};
    for (const key of ATTRIBUTION_KEYS) {
      if (parsed[key]) attribution[key] = parsed[key];
    }
    return Object.keys(attribution).length > 0 ? attribution : null;
  } catch {
    return null;
  }
}

/** Forget the kept campaign: the account it brought in exists now. */
export function clearSignupAttribution(): void {
  try {
    sessionStorage.removeItem(SIGNUP_ATTRIBUTION_KEY);
  } catch {
    // Nothing kept that could be read back either.
  }
}
