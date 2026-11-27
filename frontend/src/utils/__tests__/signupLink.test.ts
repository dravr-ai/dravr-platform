// ABOUTME: Tests the create-account link's shape: which paths are landings, and where the signup code is read from
// ABOUTME: The fragment (#code=) is read first so a shared code never has to ride a request URL or a Referer (carnet#1873)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect } from 'vitest'
import { isRegisterLandingPath, parseSignupCode } from '../signupLink'

describe('isRegisterLandingPath', () => {
  it('names /register and /signup, nothing else', () => {
    expect(isRegisterLandingPath('/register')).toBe(true)
    expect(isRegisterLandingPath('/signup')).toBe(true)
    expect(isRegisterLandingPath('/')).toBe(false)
    expect(isRegisterLandingPath('/signup/extra')).toBe(false)
  })
})

describe('parseSignupCode', () => {
  it('reads the code from the fragment', () => {
    expect(parseSignupCode('', '#code=ABC123DEF456')).toBe('ABC123DEF456')
  })

  it('still reads the code an older ?code= link carries', () => {
    expect(parseSignupCode('?code=ABC123DEF456', '')).toBe('ABC123DEF456')
  })

  it('prefers the fragment when both carry a code', () => {
    expect(parseSignupCode('?code=FROM-QUERY', '#code=FROM-FRAGMENT')).toBe('FROM-FRAGMENT')
  })

  it('falls back to the query when the fragment holds no code', () => {
    expect(parseSignupCode('?code=FROM-QUERY', '#home')).toBe('FROM-QUERY')
    expect(parseSignupCode('?code=FROM-QUERY', '#code=%20%20')).toBe('FROM-QUERY')
  })

  it('trims the code and reads a blank one as none', () => {
    expect(parseSignupCode('', '#code=%20abc%20')).toBe('abc')
    expect(parseSignupCode('?code=', '#code=')).toBeNull()
    expect(parseSignupCode('', '')).toBeNull()
  })

  it('decodes a code the fragment percent-encodes', () => {
    expect(parseSignupCode('', '#code=a%2Bb%26c')).toBe('a+b&c')
  })
})
