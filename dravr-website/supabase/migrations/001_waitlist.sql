-- ABOUTME: Waitlist table for Dravr alpha access signups
-- ABOUTME: Admins manually flip status to 'approved' to grant documentation access

create table if not exists waitlist (
  id uuid primary key default gen_random_uuid(),
  email text not null unique,
  name text,
  source text default 'website',
  created_at timestamptz default now(),
  status text default 'pending' check (status in ('pending', 'approved', 'rejected'))
);

-- Index for email lookups (auth check and duplicate detection)
create index if not exists waitlist_email_idx on waitlist (email);

-- Row Level Security: only service role can read/write (no public access)
alter table waitlist enable row level security;
