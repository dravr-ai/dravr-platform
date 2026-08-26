// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the shared @pierre/api-client featureFlags domain that replaced the duplicated web client
// ABOUTME: Pins the endpoint, the parsed map, and the one request-failure answer web and mobile share

import { describe, it, expect, vi } from 'vitest'
import type { AxiosInstance } from 'axios'
import type { MeFeaturesResponse } from '@pierre/shared-types'
import {
  createFeatureFlagsApi,
  mergeFeatureFlags,
  FALLBACK_FEATURE_FLAGS,
  FEATURE_KEYS,
} from '@pierre/api-client'

/**
 * Builds the feature flags API over an axios stub that records every GET url
 * and replays one response, so the request and the parse can both be asserted.
 */
function featureFlagsApiOver(data: MeFeaturesResponse) {
  const urls: string[] = []
  const axiosStub = {
    get: vi.fn((url: string) => {
      urls.push(url)
      return Promise.resolve({ data })
    }),
  } as unknown as AxiosInstance

  return { api: createFeatureFlagsApi(axiosStub), urls }
}

const serverResponse: MeFeaturesResponse = {
  flags: { api_tokens: true, billing_header: false },
  known: [
    { key: 'api_tokens', description: 'API Tokens settings tab', default_enabled: false },
    { key: 'billing_header', description: 'Billing header entry point', default_enabled: false },
  ],
}

describe('shared featureFlags domain', () => {
  it('reads the effective flag map from /api/me/features', async () => {
    const { api, urls } = featureFlagsApiOver(serverResponse)

    const response = await api.getMyFeatures()

    expect(urls).toEqual(['/api/me/features'])
    expect(response.flags.api_tokens).toBe(true)
    expect(response.flags.billing_header).toBe(false)
  })

  it('returns the known-flag registry alongside the map', async () => {
    const { api } = featureFlagsApiOver(serverResponse)

    const response = await api.getMyFeatures()

    expect(response.known).toHaveLength(2)
    expect(response.known[0].key).toBe('api_tokens')
    expect(response.known[0].description).toBe('API Tokens settings tab')
    expect(response.known[0].default_enabled).toBe(false)
  })

  it('layers server values over the compile defaults', () => {
    expect(mergeFeatureFlags({ api_tokens: true })).toEqual({
      api_tokens: true,
      billing_header: false,
    })
  })

  it('resolves an absent response to every flag off', () => {
    // The single request-failure answer both clients use: a network failure
    // can never reveal a gated surface.
    expect(mergeFeatureFlags(undefined)).toEqual({
      api_tokens: false,
      billing_header: false,
    })
    expect(FALLBACK_FEATURE_FLAGS.api_tokens).toBe(false)
    expect(FALLBACK_FEATURE_FLAGS.billing_header).toBe(false)
  })

  it('exposes the backend storage strings as FEATURE_KEYS', () => {
    expect(FEATURE_KEYS.apiTokens).toBe('api_tokens')
    expect(FEATURE_KEYS.billingHeader).toBe('billing_header')
  })
})
