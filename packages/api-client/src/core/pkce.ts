// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: RFC 7636 PKCE for the first-party sign-in: a random verifier, its S256 challenge, and a state
// ABOUTME: The platform supplies randomness and SHA-256 (WebCrypto on web, expo-crypto on mobile)

/** The two primitives PKCE needs, which each platform provides its own way. */
export interface PkceCrypto {
  /** `length` cryptographically random bytes. */
  randomBytes(length: number): Uint8Array;
  /** The SHA-256 digest of `input`'s ASCII bytes. */
  sha256(input: string): Promise<Uint8Array>;
}

/** A PKCE verifier, the challenge sent with the authorization request, and its `state`. */
export interface PkcePair {
  codeVerifier: string;
  codeChallenge: string;
  state: string;
}

/** 32 random bytes: a 43-character verifier, the shortest RFC 7636 §4.1 allows. */
const VERIFIER_BYTES = 32;

/** 16 random bytes of `state`, enough that a forged callback cannot guess it. */
const STATE_BYTES = 16;

const BASE64URL_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';

/** Unpadded base64url (RFC 4648 §5), written out so Hermes needs no `btoa`. */
export function base64UrlEncode(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i];
    const b1 = i + 1 < bytes.length ? bytes[i + 1] : 0;
    const b2 = i + 2 < bytes.length ? bytes[i + 2] : 0;
    const triple = (b0 << 16) | (b1 << 8) | b2;
    out += BASE64URL_ALPHABET[(triple >> 18) & 63];
    out += BASE64URL_ALPHABET[(triple >> 12) & 63];
    if (i + 1 < bytes.length) out += BASE64URL_ALPHABET[(triple >> 6) & 63];
    if (i + 2 < bytes.length) out += BASE64URL_ALPHABET[triple & 63];
  }
  return out;
}

/** A fresh verifier, its S256 challenge (RFC 7636 §4.2) and a `state`. */
export async function createPkcePair(crypto: PkceCrypto): Promise<PkcePair> {
  const codeVerifier = base64UrlEncode(crypto.randomBytes(VERIFIER_BYTES));
  const codeChallenge = base64UrlEncode(await crypto.sha256(codeVerifier));
  const state = base64UrlEncode(crypto.randomBytes(STATE_BYTES));
  return { codeVerifier, codeChallenge, state };
}
