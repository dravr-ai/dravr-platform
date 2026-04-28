# Stripe Webhook Smoke Test

Runbook for validating the Phase 5 Stripe integration end-to-end after
a fresh tenant subscription is created through the UI.

## Prerequisites

- Stripe CLI installed (`brew install stripe/stripe-cli/stripe`)
- Stripe test-mode keys in `.envrc`:
  - `STRIPE_SECRET_KEY`
  - `STRIPE_WEBHOOK_SECRET`
  - `STRIPE_PRICE_ID_STARTER`
  - `STRIPE_PRICE_ID_PROFESSIONAL`
  - `STRIPE_PRICE_ID_ENTERPRISE`
  - `STRIPE_USAGE_METER_ID`
- `pierre-server` running on `:8081`
- Reset dev database: `./bin/reset-dev-db.sh`

## Steps

### Terminal 1 — server

```bash
./bin/start-server.sh
```

### Terminal 2 — webhook forwarder

```bash
stripe login
stripe listen --forward-to localhost:8081/webhooks/stripe
```

Copy the displayed "webhook signing secret" into `STRIPE_WEBHOOK_SECRET`
in `.envrc`, then `direnv allow` and restart `pierre-server`.

### Terminal 3 — fire events

```bash
stripe trigger customer.subscription.created
stripe trigger customer.subscription.updated
stripe trigger invoice.payment_failed
stripe trigger charge.refunded
```

## Verification

- Admin UI → User Detail → Usage tab: tier reflects subscription state.
- SQL: `SELECT * FROM subscriptions ORDER BY updated_at DESC LIMIT 5;`
- Logs: `grep stripe_webhook logs/pierre.log` — no errors.
- Downgrade path: trigger `invoice.payment_failed`, wait 7 days (or
  override the grace-period env `STRIPE_GRACE_PERIOD_DAYS=0` and fire
  a second `invoice.payment_failed`), then re-check `users.tier` is
  `starter`.

## Overage cron

The monthly overage cron reads `llm_usage.cost_usd` for each tenant at
`00:10 UTC` on the 1st of each month and reports usage via the Stripe
Meters API keyed by `STRIPE_USAGE_METER_ID`. Force an immediate run in
dev:

```bash
cargo run --bin pierre-cli -- billing run-overage --period YYYY-MM
```

## Rollback

Switch `STRIPE_WEBHOOK_SECRET` to an empty string to reject incoming
webhook POSTs at the signature-verification step without disabling the
rest of the server. Queued events stay in Stripe for replay once the
secret is restored.
