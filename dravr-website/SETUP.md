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
5. Copy your project keys from **Settings → API**:
   - `URL` → `PUBLIC_SUPABASE_URL`
   - `anon public` key → `PUBLIC_SUPABASE_ANON_KEY`
   - `service_role` key → `SUPABASE_SERVICE_ROLE_KEY` (keep this secret)

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

## 3. Cloudflare Pages deployment

1. In the Cloudflare dashboard, go to **Workers & Pages → Create → Pages**.
2. Connect your GitHub repository.
3. Set build settings:
   - **Build command:** `cd dravr-website && bun install && bun run build`
   - **Build output directory:** `dravr-website/dist`
   - **Root directory:** `/` (leave as repo root)
4. Add environment variables (Settings → Environment Variables):
   - `PUBLIC_SUPABASE_URL` — variable
   - `PUBLIC_SUPABASE_ANON_KEY` — variable
   - `PUBLIC_SITE_URL` — variable, set to `https://dravr.ai` (or preview URL for preview envs)
   - `PUBLIC_TURNSTILE_SITE_KEY` — variable
   - `PUBLIC_GOOGLE_AUTH_ENABLED` — variable, `true` or `false`
   - `SUPABASE_SERVICE_ROLE_KEY` — **secret**
   - `TURNSTILE_SECRET_KEY` — **secret**
5. Deploy.

---

## 4. Custom domain

1. In Cloudflare Pages → your project → **Custom domains** → Add `dravr.ai`.
2. Since the domain is already on Cloudflare, DNS is configured automatically.
3. Update Supabase Site URL and redirect URLs to `https://dravr.ai`.

---

## 5. Approving waitlist users

To grant a user access to the alpha documentation:

```sql
UPDATE waitlist
SET status = 'approved'
WHERE email = 'user@example.com';
```

Run this in the Supabase SQL Editor. The user can then log in at `/docs/login` using a magic link.
