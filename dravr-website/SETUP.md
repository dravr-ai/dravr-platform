# Dravr Website — Setup

## Prerequisites

- Bun installed (`brew install bun`)
- A [Supabase](https://supabase.com) account (free tier)
- A [Cloudflare](https://cloudflare.com) account (free tier)
- The `dravr.ai` domain pointed to Cloudflare (or a temporary `*.pages.dev` subdomain)

---

## 1. Supabase project setup

1. Create a new project at [app.supabase.com](https://app.supabase.com).
2. Go to **SQL Editor** and run the contents of each file in `supabase/migrations/`
   in numeric order (`001_waitlist.sql`, then `002_waitlist_policies.sql`).
3. Go to **Authentication → Providers** and ensure **Email** is enabled.
4. Go to **Authentication → URL Configuration**:
   - Set **Site URL** to `https://dravr.ai` (or your Cloudflare Pages URL during testing).
   - Set **Redirect URLs** to *exactly* the URLs you need — no wildcards:
     - `https://dravr.ai/docs/auth/callback`
     - `http://localhost:4321/docs/auth/callback` (for local dev only)
   - A permissive allowlist here lets an attacker intercept magic-link tokens —
     audit the list before going public.
5. Copy your project keys from **Settings → API Keys**. Use the
   **"Publishable and secret API keys"** tab — the legacy `anon` /
   `service_role` keys still work during the transition, but new projects
   should use the new key format:
   - `Project URL` → `PUBLIC_SUPABASE_URL`
   - `Publishable key` (`sb_publishable_...`) → `PUBLIC_SUPABASE_PUBLISHABLE_KEY`
     — safe to expose in browser bundles; subject to RLS.
   - `Secret key` (`sb_secret_...`) → `SUPABASE_SECRET_KEY` — server-only,
     bypasses RLS. Required for the middleware waitlist-approval lookup
     and for `/api/waitlist` inserts (the `waitlist` table is locked to
     the `service_role` role by migration `002_waitlist_policies.sql`).

---

## 1b. Cloudflare Turnstile setup (bot protection on the waitlist form)

Turnstile is Cloudflare's free, privacy-friendly CAPTCHA replacement. The
waitlist API rejects any signup that does not include a valid Turnstile token.

1. In the Cloudflare dashboard, go to **Turnstile → Add site**.
2. Fill in the form:
   - **Site name:** `dravr.ai`
   - **Domains:** add `dravr.ai` *and* `localhost` (so local dev works)
   - **Widget mode:** **Managed** — shows an interactive challenge only when
     traffic looks suspicious; silent for normal visitors.
3. Copy the generated keys:
   - **Site Key** → `PUBLIC_TURNSTILE_SITE_KEY` (safe to expose in HTML)
   - **Secret Key** → `TURNSTILE_SECRET_KEY` (server-only, keep secret)
4. Add both to your `.env` for local dev. You can use Cloudflare's always-pass
   test keys instead of real ones locally (already wired up in `.env.example`):
   ```
   PUBLIC_TURNSTILE_SITE_KEY=1x00000000000000000000AA
   TURNSTILE_SECRET_KEY=1x0000000000000000000000000000000AA
   ```
5. In Cloudflare Pages → your project → **Settings → Environment variables**,
   add the real production keys:
   - `PUBLIC_TURNSTILE_SITE_KEY` — **Environment variable** (not a secret)
   - `TURNSTILE_SECRET_KEY` — **Encrypted/Secret**

---

## 2. Local development

```bash
cd dravr-website

# Install dependencies
bun install

# Copy and fill in env vars
cp .env.example .env
# Edit .env with your Supabase keys

# Start dev server
bun dev
# → http://localhost:4321
```

---

## 3. Cloudflare Workers deployment

The site deploys as a Cloudflare **Worker with Static Assets** — the Astro
Cloudflare adapter emits `dist/_worker.js/` (SSR entry) alongside the static
files, and Cloudflare serves `dist/*` as assets while routing dynamic
requests through the Worker. Config lives in `wrangler.jsonc`; see the
[Workers best practices](https://developers.cloudflare.com/workers/best-practices/workers-best-practices/)
for the reasoning behind the `nodejs_compat` flag, `observability`, and
Smart Placement.

### Secrets hygiene

**Never put `*_SECRET_KEY` values in `wrangler.jsonc`** — it's committed
to git. Use `wrangler secret put NAME` instead. Public build-time vars
(`PUBLIC_*`) are inlined into the bundle at `astro build`, so they must
exist in the build environment **at build time**, not just at runtime.

### First-time deploy

1. Authenticate wrangler once on your machine:
   ```bash
   cd dravr-website
   bunx wrangler login
   ```
2. Set the production secrets (one-time, interactive prompts):
   ```bash
   bunx wrangler secret put SUPABASE_SECRET_KEY
   bunx wrangler secret put TURNSTILE_SECRET_KEY
   ```
3. Export the public build-time vars before running the build. Either put
   them in `.env` (loaded by Astro) or export them in your shell:
   - `PUBLIC_SUPABASE_URL`
   - `PUBLIC_SUPABASE_PUBLISHABLE_KEY`
   - `PUBLIC_SITE_URL` (`https://dravr.ai` for production)
   - `PUBLIC_TURNSTILE_SITE_KEY`
   - `PUBLIC_GOOGLE_AUTH_ENABLED`
4. Deploy:
   ```bash
   bun run deploy
   ```
   The first deploy will prompt to create the `dravr-website` Worker.
   Subsequent deploys are silent.

### Useful commands

```bash
bun run deploy:dry-run   # build + validate wrangler.jsonc without publishing
bun run cf:tail          # live-tail production Worker logs
bunx wrangler dev        # run the built Worker locally on :8787
```

---

## 4. Custom domain

1. In the Cloudflare dashboard, go to **Workers & Pages → dravr-website →
   Settings → Domains & Routes → Add** → `dravr.ai`.
2. Since the `dravr.ai` zone is already on Cloudflare, DNS is configured
   automatically — no external DNS step.
3. Update Supabase Site URL and redirect URLs to `https://dravr.ai`
   (see §1, step 4).

---

## 5. Approving waitlist users

To grant a user access to the alpha documentation:

```sql
UPDATE waitlist
SET status = 'approved'
WHERE email = 'user@example.com';
```

Run this in the Supabase SQL Editor. The user can then log in at `/docs/login` using a magic link.
