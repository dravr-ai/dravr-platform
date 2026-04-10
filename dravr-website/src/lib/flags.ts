// ABOUTME: Feature-flag helpers for the Dravr website
// ABOUTME: Parses string env vars (from Cloudflare vars) into booleans with sane fallbacks

const TRUTHY = new Set(['true', '1', 'yes', 'on']);

// Converts a stringly-typed env flag into a boolean.
// Accepts common truthy spellings so a case mismatch (e.g. "True") doesn't
// silently disable a feature the way a strict `=== 'true'` check would.
export function isFlagEnabled(value: string | undefined): boolean {
  return value !== undefined && TRUTHY.has(value.trim().toLowerCase());
}
