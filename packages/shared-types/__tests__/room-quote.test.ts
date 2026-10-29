// ABOUTME: Unit tests for the room-quote refusal reasons — the wire values a refused quote is named by (carnet#1249)
// ABOUTME: A client clears the quote and says it can no longer be used only for these, never for a coaching-gate refusal

import { describe, it, expect } from 'vitest';
import { isRoomQuoteRefusalReason } from '../src/room-quote';

describe('isRoomQuoteRefusalReason', () => {
  // Wire values: `details.reason` of the refusals routes/chat/room_quote.rs answers with.
  it('names each refusal the quote itself is answered with', () => {
    expect(isRoomQuoteRefusalReason('quote_not_allowed')).toBe(true);
    expect(isRoomQuoteRefusalReason('quote_not_found')).toBe(true);
    expect(isRoomQuoteRefusalReason('quote_withheld')).toBe(true);
    expect(isRoomQuoteRefusalReason('quote_not_athlete')).toBe(true);
  });

  it("is false for the coaching gates' refusals, which are about the thread, not the quote", () => {
    expect(isRoomQuoteRefusalReason('coach_sharing_revoked')).toBe(false);
    expect(isRoomQuoteRefusalReason('not_group_coach')).toBe(false);
    expect(isRoomQuoteRefusalReason('several_tenants')).toBe(false);
  });

  it('is false for anything that is not a reason', () => {
    expect(isRoomQuoteRefusalReason(undefined)).toBe(false);
    expect(isRoomQuoteRefusalReason(null)).toBe(false);
    expect(isRoomQuoteRefusalReason('QUOTE_WITHHELD')).toBe(false);
    expect(isRoomQuoteRefusalReason(403)).toBe(false);
  });
});
