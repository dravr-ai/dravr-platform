# Dravr platform — environment

Appended to pi's default system prompt. These are the constant notices this repo's Claude
Code `SessionStart` hooks echoed; the dynamic checks live in `.pi/extensions/dravr-session.ts`.

## Ports

`8081` = Pierre Server (RESERVED — never start anything else on it). `8082` = Expo/Metro.
Use `bun start` for mobile dev.

## Dev environment

When testing is needed, run `./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`
(debug build by default, `--release` for optimized). It resets the DB, seeds
admin/coaches/demo/social/mobility/synthetic data, and starts Pierre (8081), the Vite
frontend (5173) and Expo (8082). NEVER start the server another way when login or user
data is needed.

Seeded dev credentials: `admin@example.com` / `AdminPassword123`,
`webtest@pierre.dev` / `WebTest123!`, `mobiletest@pierre.dev` / `MobileTest1234`,
`alice@acme.com` / `DemoUser123!`. The admin token is written to `logs/admin-token.txt`.

## Design system

Use `/skill:design-review` for UI validation before considering frontend work done.
