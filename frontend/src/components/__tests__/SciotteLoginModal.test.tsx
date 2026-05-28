// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for SciotteLoginModal's server-driven timeout copy
// ABOUTME: Covers formatTimeout (minutes vs seconds) used by the progress text

import { describe, it, expect } from 'vitest'
import { formatTimeout } from '../sciotteLoginCopy'

describe('formatTimeout', () => {
  it('renders short budgets in seconds', () => {
    expect(formatTimeout(30)).toBe('30 seconds')
    expect(formatTimeout(89)).toBe('89 seconds')
  })

  it('collapses budgets >= 90s into whole minutes', () => {
    expect(formatTimeout(240)).toBe('4 minutes')
    expect(formatTimeout(210)).toBe('4 minutes')
    expect(formatTimeout(180)).toBe('3 minutes')
  })

  it('switches units at the 90s boundary', () => {
    expect(formatTimeout(90)).toBe('2 minutes')
    expect(formatTimeout(60)).toBe('60 seconds')
  })
})
