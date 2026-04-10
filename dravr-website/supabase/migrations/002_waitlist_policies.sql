-- ABOUTME: Explicit RLS policies for the waitlist table
-- ABOUTME: Documents intent — only service_role has access; anon/authenticated are fully denied

-- The default behaviour of an RLS-enabled table with zero policies is "deny
-- everything for non-superuser roles", which keeps us safe today. These
-- explicit statements exist so a future contributor who adds a permissive
-- policy for `authenticated` cannot accidentally open up the waitlist to
-- end users without also noticing the belt-and-braces revokes below.

-- Service role: full access (bypasses RLS anyway, but we make the
-- intended access shape obvious in the policy list).
create policy "waitlist_service_role_all"
  on waitlist
  for all
  to service_role
  using (true)
  with check (true);

-- Belt-and-braces: strip any inherited grants from anon/authenticated.
-- This is redundant with "RLS enabled + no policy" but prevents a
-- `grant select` added elsewhere from silently taking effect.
revoke all on waitlist from anon;
revoke all on waitlist from authenticated;
