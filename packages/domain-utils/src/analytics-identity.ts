// ABOUTME: The PostHog distinct id of a user: the lowercase hex SHA-256 of the user UUID string
// ABOUTME: The same value as the server's pierre_services::analytics::hash_id, so web, phone and server events share one person

/**
 * SHA-256 of `data`. Each platform supplies its own: WebCrypto on the web,
 * expo-crypto on the phone (Hermes has no WebCrypto).
 */
export type Sha256Digest = (data: Uint8Array<ArrayBuffer>) => Promise<ArrayBuffer>;

/**
 * The `distinct_id` PostHog sees for a user, on every surface.
 *
 * It is the lowercase hex SHA-256 of the UTF-8 bytes of the user id, with no
 * prefix: exactly what the server's notify-to-PostHog sink derives with
 * `pierre_services::analytics::hash_id`. A client that identified itself any
 * other way would be a second person to PostHog, and no funnel could join the
 * events it sent with the ones the server sent for the same athlete.
 *
 * @param userId The user UUID in the form the server holds it (hyphenated, lowercase).
 * @param sha256 The platform's SHA-256.
 */
export async function analyticsDistinctId(userId: string, sha256: Sha256Digest): Promise<string> {
  // `from` copies into a plain ArrayBuffer, which WebCrypto and expo-crypto both accept as a BufferSource.
  const digest = await sha256(Uint8Array.from(new TextEncoder().encode(userId)));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}
