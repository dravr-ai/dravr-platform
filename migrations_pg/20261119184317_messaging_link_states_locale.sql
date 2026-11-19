-- ABOUTME: Records the language an in-chat linking flow speaks, chosen when the flow opens
-- ABOUTME: A flow's prompts read it, so a sender with no account never sees the language switch mid-way
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- Every prompt a sender read before linking an account was in the default
-- locale, French, whatever language they spoke (carnet#1669). The locale is
-- now chosen from what the channel tells us about the sender (a WhatsApp
-- number's country calling code) when their flow opens: the in-chat OTP flow
-- on its first prompt, a pairing code when the sender redeems it. Every later
-- prompt of that flow reads it from here rather than choosing again. NULL on
-- a state no chat flow has opened (a pairing code not yet redeemed, an OAuth
-- link, a link-page code).

ALTER TABLE messaging_link_states ADD COLUMN IF NOT EXISTS locale TEXT;
