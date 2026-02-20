// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect } from 'vitest'
import { computeWarningState } from '../useUsageStatus'
import type { UsageStatusResponse, LimitCheckResult } from '../../services/api/usage'

function makeLimitCheck(overrides: Partial<LimitCheckResult> = {}): LimitCheckResult {
  return {
    allowed: true,
    current: 0,
    limit: 100,
    warning: false,
    burst_zone: false,
    resets_at: '2026-02-21T00:00:00Z',
    ...overrides,
  }
}

function makeUsageResponse(overrides: Partial<{
  dailyMessages: Partial<LimitCheckResult>;
  dailyTokens: Partial<LimitCheckResult>;
  weeklyMessages: Partial<LimitCheckResult>;
}> = {}): UsageStatusResponse {
  return {
    daily: {
      messages: makeLimitCheck(overrides.dailyMessages),
      tokens: makeLimitCheck(overrides.dailyTokens),
      tool_calls: makeLimitCheck(),
    },
    weekly: {
      messages: makeLimitCheck(overrides.weeklyMessages),
      tokens: makeLimitCheck(),
      tool_calls: makeLimitCheck(),
    },
    resources: {
      conversations: 5,
      max_conversations: 50,
      coaches: 3,
      max_coaches: 10,
    },
  }
}

describe('computeWarningState', () => {
  it('should return none level when data is undefined', () => {
    const result = computeWarningState(undefined)

    expect(result.level).toBe('none')
    expect(result.sendDisabled).toBe(false)
    expect(result.message).toBe('')
    expect(result.triggerCounter).toBeNull()
  })

  it('should return none level when all counters are within limits', () => {
    const data = makeUsageResponse()
    const result = computeWarningState(data)

    expect(result.level).toBe('none')
    expect(result.sendDisabled).toBe(false)
    expect(result.message).toBe('')
  })

  it('should return warning level when a counter is in warning zone', () => {
    const data = makeUsageResponse({
      dailyMessages: { warning: true, current: 80, limit: 100 },
    })
    const result = computeWarningState(data)

    expect(result.level).toBe('warning')
    expect(result.sendDisabled).toBe(false)
    expect(result.message).toContain('80%')
    expect(result.message).toContain('daily messages')
    expect(result.message).toContain('80/100')
  })

  it('should return burst level when a counter is in burst zone', () => {
    const data = makeUsageResponse({
      dailyTokens: { burst_zone: true, current: 95, limit: 100 },
    })
    const result = computeWarningState(data)

    expect(result.level).toBe('burst')
    expect(result.sendDisabled).toBe(false)
    expect(result.message).toContain('burst zone')
    expect(result.message).toContain('daily tokens')
  })

  it('should return blocked level when a counter is not allowed', () => {
    const data = makeUsageResponse({
      dailyMessages: { allowed: false, current: 100, limit: 100 },
    })
    const result = computeWarningState(data)

    expect(result.level).toBe('blocked')
    expect(result.sendDisabled).toBe(true)
    expect(result.message).toContain('limit reached')
    expect(result.message).toContain('Daily messages')
  })

  it('should prioritize blocked over burst', () => {
    const data = makeUsageResponse({
      dailyMessages: { burst_zone: true, current: 95, limit: 100 },
      dailyTokens: { allowed: false, current: 100, limit: 100 },
    })
    const result = computeWarningState(data)

    expect(result.level).toBe('blocked')
    expect(result.sendDisabled).toBe(true)
  })

  it('should prioritize burst over warning', () => {
    const data = makeUsageResponse({
      dailyMessages: { warning: true, current: 80, limit: 100 },
      dailyTokens: { burst_zone: true, current: 95, limit: 100 },
    })
    const result = computeWarningState(data)

    expect(result.level).toBe('burst')
  })

  it('should include resets_at in the result', () => {
    const resetTime = '2026-02-21T12:00:00Z'
    const data = makeUsageResponse({
      dailyMessages: { warning: true, current: 80, limit: 100, resets_at: resetTime },
    })
    const result = computeWarningState(data)

    expect(result.resetsAt).toBe(resetTime)
  })

  it('should set triggerCounter to the worst counter', () => {
    const data = makeUsageResponse({
      dailyMessages: { allowed: false, current: 100, limit: 100 },
    })
    const result = computeWarningState(data)

    expect(result.triggerCounter).not.toBeNull()
    expect(result.triggerCounter?.current).toBe(100)
    expect(result.triggerCounter?.allowed).toBe(false)
  })

  it('should handle zero limit gracefully', () => {
    const data = makeUsageResponse({
      dailyMessages: { warning: true, current: 0, limit: 0 },
    })
    const result = computeWarningState(data)

    expect(result.level).toBe('warning')
    expect(result.message).toContain('0%')
  })

  it('should check weekly messages counter', () => {
    const data = makeUsageResponse({
      weeklyMessages: { allowed: false, current: 500, limit: 500 },
    })
    const result = computeWarningState(data)

    expect(result.level).toBe('blocked')
    expect(result.message).toContain('Weekly messages')
  })
})
