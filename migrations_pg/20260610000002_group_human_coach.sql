-- ABOUTME: Adds human coach attachment to coaching groups + coach invite kind
-- ABOUTME: coach_user_id links a professional coach user; group_invites.kind gates redemption

-- A human professional coach (a Dravr user) overseeing the group. NULL until
-- a coach redeems a coach-kind invite. Distinct from coaching_groups.coach_id,
-- which is the AI coach persona.
ALTER TABLE coaching_groups ADD COLUMN IF NOT EXISTS coach_user_id UUID REFERENCES users(id);
CREATE INDEX IF NOT EXISTS idx_coaching_groups_coach_user ON coaching_groups(coach_user_id);

-- What redeeming an invite grants: 'member' (athlete, default — matches every
-- existing invite) or 'coach' (attaches the redeemer as coach_user_id).
-- Validated in the app layer via GroupInviteKind::from_str_opt.
ALTER TABLE group_invites ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'member';
