# Dravr Website — Setup

## Prerequisites

- Bun installed (`brew install bun`)
- A [Supabase](https://supabase.com) account (free tier)
- A [Cloudflare](https://cloudflare.com) account (free tier)
- The `dravr.ai` domain pointed to Cloudflare (or a temporary `*.pages.dev` subdomain)

---

## 1. Supabase project setup

1. Create a new project at [app.supabase.com](https://app.supabase.com).
2. Go to **SQL Editor** and run the contents of `supabase/migrations/001_waitlist.sql`.
3. Go to **Authentication → Providers** and ensure **Email** is enabled.
4. Go to **Authentication → URL Configuration**:
   - Set **Site URL** to `https://dravr.ai` (or your Cloudflare Pages URL during testing)
   - Add `https://dravr.ai/docs/auth/callback` to **Redirect URLs**
5. Copy your project keys from **Settings → API**:
   - `URL` → `SUPABASE_URL` and `PUBLIC_SUPABASE_URL`
   - `anon public` key → `PUBLIC_SUPABASE_ANON_KEY`
   - `service_role` key → `SUPABASE_SERVICE_ROLE_KEY` (keep this secret)

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
   - `PUBLIC_SUPABASE_URL`
   - `PUBLIC_SUPABASE_ANON_KEY`
   - `SUPABASE_URL`
   - `SUPABASE_SERVICE_ROLE_KEY`
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
