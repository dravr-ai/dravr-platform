// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how the landing attribution (utm_*, dravr.ai button) is read from either side of the hash, kept and spent
// ABOUTME: A blocked storage or a tampered record never throws and never sends anything but the attribution fields

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import {
  SIGNUP_ATTRIBUTION_KEY,
  captureSignupAttribution,
  clearSignupAttribution,
  readAttributionParameters,
  readSignupAttribution,
} from '../signupAttribution'

describe('readAttributionParameters', () => {
  it('reads the query before the hash', () => {
    expect(readAttributionParameters({ search: '?utm_source=newsletter&utm_medium=email&utm_campaign=fall', hash: '#home' })).toEqual({
      utm_source: 'newsletter',
      utm_medium: 'email',
      utm_campaign: 'fall',
    })
  })

  it('reads the query after the hash', () => {
    expect(readAttributionParameters({ search: '', hash: '#/signup?utm_source=strava-club&utm_campaign=fall' })).toEqual({
      utm_source: 'strava-club',
      utm_campaign: 'fall',
    })
  })

  it('takes the query before the hash first, and fills the rest from after it', () => {
    expect(readAttributionParameters({ search: '?utm_source=first', hash: '#home?utm_source=second&utm_medium=social' })).toEqual({
      utm_source: 'first',
      utm_medium: 'social',
    })
  })

  it('reads the button a dravr.ai create-account link names, beside the code it may carry', () => {
    expect(
      readAttributionParameters({ search: '?ref=dravr.ai&placement=hero&lang=fr&code=abc', hash: '' }),
    ).toEqual({ ref: 'dravr.ai', placement: 'hero', lang: 'fr' })
  })

  it('reads the term and content tags a campaign link forwards', () => {
    expect(readAttributionParameters({ search: '?utm_source=newsletter&utm_content=footer&utm_term=coach', hash: '' })).toEqual({
      utm_source: 'newsletter',
      utm_content: 'footer',
      utm_term: 'coach',
    })
  })

  it('ignores blank values, other parameters and an address without a campaign', () => {
    expect(readAttributionParameters({ search: '?utm_source=%20%20&code=abc', hash: '#home' })).toBeNull()
    expect(readAttributionParameters({ search: '', hash: '' })).toBeNull()
  })

  it('trims a value and drops an overlong one rather than cutting it', () => {
    const long = 'x'.repeat(201)
    const read = readAttributionParameters({ search: `?utm_source=%20spaced%20&utm_campaign=${long}`, hash: '' })
    expect(read).toEqual({ utm_source: 'spaced' })
  })
})

describe('signup attribution storage', () => {
  beforeEach(() => {
    sessionStorage.clear()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('keeps the landing campaign until it is spent', () => {
    captureSignupAttribution({ search: '?utm_source=newsletter&utm_campaign=fall', hash: '' })

    expect(readSignupAttribution()).toEqual({ utm_source: 'newsletter', utm_campaign: 'fall' })

    clearSignupAttribution()
    expect(readSignupAttribution()).toBeNull()
    expect(sessionStorage.getItem(SIGNUP_ATTRIBUTION_KEY)).toBeNull()
  })

  it('keeps the campaign when a later address names none, and replaces it when a newer link does', () => {
    captureSignupAttribution({ search: '?utm_source=first&utm_medium=email', hash: '' })
    captureSignupAttribution({ search: '', hash: '#chat' })
    expect(readSignupAttribution()).toEqual({ utm_source: 'first', utm_medium: 'email' })

    captureSignupAttribution({ search: '?utm_source=second', hash: '' })
    expect(readSignupAttribution()).toEqual({ utm_source: 'second' })
  })

  it('keeps a dravr.ai button link the same way, and a newer campaign link replaces it', () => {
    captureSignupAttribution({ search: '?ref=dravr.ai&placement=cohort&lang=en', hash: '' })
    expect(readSignupAttribution()).toEqual({ ref: 'dravr.ai', placement: 'cohort', lang: 'en' })

    captureSignupAttribution({ search: '?utm_source=newsletter', hash: '' })
    expect(readSignupAttribution()).toEqual({ utm_source: 'newsletter' })
  })

  it('reads only the attribution fields from a stored record, and nothing from a malformed one', () => {
    sessionStorage.setItem(
      SIGNUP_ATTRIBUTION_KEY,
      JSON.stringify({ utm_source: 'x', placement: 'hero', password: 'nope' }),
    )
    expect(readSignupAttribution()).toEqual({ utm_source: 'x', placement: 'hero' })

    sessionStorage.setItem(SIGNUP_ATTRIBUTION_KEY, JSON.stringify({ utm_source: 42 }))
    expect(readSignupAttribution()).toBeNull()

    sessionStorage.setItem(SIGNUP_ATTRIBUTION_KEY, '{not json')
    expect(readSignupAttribution()).toBeNull()
  })

  it('never throws when storage is blocked', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('blocked', 'SecurityError')
    })
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('blocked', 'SecurityError')
    })
    vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
      throw new DOMException('blocked', 'SecurityError')
    })

    expect(() => captureSignupAttribution({ search: '?utm_source=x', hash: '' })).not.toThrow()
    expect(readSignupAttribution()).toBeNull()
    expect(() => clearSignupAttribution()).not.toThrow()
  })
})
