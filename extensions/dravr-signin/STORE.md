<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Copyright (c) 2026 dravr.ai -->
<!-- ABOUTME: What to paste into the Chrome Web Store, Edge Add-ons and addons.mozilla.org listings for the sign-in extension -->
<!-- ABOUTME: Kept beside the code so a listing never says something the extension does not do (carnet#1617) -->

# Store listing: Dravr sign-in

The package is `./package.sh`, which writes `dist/dravr-signin-<version>.zip`. The same zip goes to
all three stores. Upload the images in `store/`.

**Keep this file true.** A reviewer compares the listing with the code. Change this file in the same
commit as any change to a permission, to what the extension reads, or to where that data goes. The
privacy policy section it links ([dravr.ai/privacy#signin-extension](https://dravr.ai/privacy#signin-extension),
in the `dravr-website` repo) must say the same.

## Listing

| Field | Value |
|---|---|
| Name | Dravr sign-in (localized from `_locales/`) |
| Summary (132 characters max) | Sign in to Garmin, Strava, TrainingPeaks or COROS in your own browser and connect that account to Dravr. |
| Category | Chrome: Productivity · Tools. Edge: Productivity. Firefox: Other |
| Language | English, plus French, Spanish, German and Portuguese from `_locales/` |
| Homepage | https://dravr.ai |
| Support | privacy@dravr.ai |
| Privacy policy | https://dravr.ai/privacy#signin-extension |
| Visibility | Unlisted for the alpha: reachable by its link only |
| Screenshot (1280×800) | `store/screenshot-notice-1280x800.png` |
| Small promo tile (440×280) | `store/promo-tile-440x280.png` |
| Icon (128×128) | `icons/icon-128.png` |

### Description

> Dravr is an AI training partner that reads your training from Garmin, Strava, TrainingPeaks and
> COROS. Some of these services block a sign-in made on your behalf. With this extension you sign in
> yourself, in a normal tab of your own browser, and hand that sign-in to Dravr.
>
> How it works:
> 1. In Dravr (app.dravr.ai), choose "Continue with Garmin", for example.
> 2. The service's own sign-in page opens in a new tab. Sign in as you always do, two-step
>    verification included.
> 3. Once you are signed in, the extension passes that service's session to Dravr and closes the tab.
>    Dravr then imports your activities.
>
> What it reads: only the cookies of the service you chose, and only after you finish signing in to
> it from Dravr. It reads no other site, no other tab and nothing you type. Where it goes: straight
> to Dravr over an encrypted connection, and nowhere else. Dravr stores the session encrypted and
> uses it only to fetch your data from that service. The extension keeps no copy.
>
> Before its first use, the extension shows what it reads and asks you to accept. Disconnect the
> service in Dravr to delete the stored session, and remove the extension to stop it.

## Chrome Web Store: Privacy practices tab

**Single purpose**

> Lets an athlete sign in to their own Garmin, Strava, TrainingPeaks or COROS account in their own
> browser and hand that sign-in to Dravr, so Dravr can import their training.

**Permission justifications**

| Permission | Justification |
|---|---|
| `cookies` | Reads the session cookies of the one service the athlete signs in to (garmin.com, strava.com, trainingpeaks.com or coros.com), once they finish signing in, and hands them to Dravr. That hand-over is the extension's single purpose. |
| `storage` | Remembers that the athlete accepted the notice shown before the first sign-in. Nothing else is stored. |
| Host permission `*.garmin.com`, `*.strava.com`, `*.trainingpeaks.com`, `*.coros.com` | Lets the extension see when the sign-in tab it opened reaches the service's signed-in page, and read that service's cookies. No other site is requested. |
| Content script on `app.dravr.ai` | Lets the Dravr web app ask for a sign-in and receive the result (`bridge.js`). It is declared in the manifest, runs on Dravr's own app only, and needs no host permission: the extension can read no cookie of Dravr's. |
| Remote code | No. Every script ships in the package. |

**Data usage**

- Collected: **Authentication information** (the chosen service's session cookies). Nothing else:
  no personally identifiable information, health data, location, web history, user activity or
  website content.
- Certify all three: not sold or transferred to third parties outside the approved use cases; not
  used or transferred for purposes unrelated to the item's single purpose; not used or transferred
  to determine creditworthiness or for lending purposes.

## Edge Add-ons

Microsoft Partner Center takes the same zip, description, images and privacy policy URL. In "Does
your extension access, collect or transmit personal information?" answer **Yes**: authentication
information, sent to Dravr only, as described in the privacy policy.

## addons.mozilla.org (Firefox)

- Upload the same zip. AMO reads `browser_specific_settings.gecko`: ID `dravr-signin@dravr.ai`,
  Firefox 128 or later, and the `authenticationInfo` data collection the install prompt shows.
- Distribution: **On this site** for a listed add-on, or **On your own** to sign it for
  self-distribution. Either way, Mozilla reviews it.
- Source code: not required. The package is the unminified source.
- Reviewer notes:

> Opened from https://app.dravr.ai: a user signed in to Dravr chooses "Continue with Garmin" in the
> Connect window. The extension opens Garmin's own sign-in page in a tab. When that tab reaches the
> signed-in page, it reads the garmin.com cookies and posts them to Dravr through bridge.js, the
> content script on app.dravr.ai. The background refuses any port that does not come from an
> app.dravr.ai tab. The notice (notice.html) is shown on install and before the first sign-in.
> Unit tests: `bun test extensions/dravr-signin` in https://github.com/dravr-ai/dravr-platform.
