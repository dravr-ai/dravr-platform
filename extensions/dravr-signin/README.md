# Dravr sign-in extension

Signs the athlete in to Garmin, Strava, TrainingPeaks or COROS **in their own
browser** and hands that provider's session to Dravr (carnet#1617).

Since 2026-11-16, Garmin puts a reCAPTCHA in front of every sign-in that comes
from a cloud browser, sciotte's included, but still serves reads to a session
it signed in. So the sign-in runs where the provider sees a person: the
athlete's own Chrome. sciotte then reads with that session as after any sign-in.

## How it works

1. The app (`frontend/src/utils/signinExtension.ts`) talks to the extension
   through its content script, `bridge.js`, which runs on https://app.dravr.ai
   only. The two exchange `window.postMessage` messages on the page's own
   origin, under the channel `dravr-signin`. The script relays each exchange
   to a port into the extension's background. This is the one channel Chrome
   and Firefox share: Firefox and Safari have no `externally_connectable`. The
   background closes any port that does not come from a tab of app.dravr.ai.
2. The app asks Dravr for the provider's entry (`GET /api/providers/browser-signin`,
   served by sciotte from its provider configs). The entry holds the sign-in
   page, the URLs that say the athlete is signed in, and the cookie domain.
3. The extension opens the sign-in page in a real tab. The athlete signs in as
   usual, 2FA included.
4. When the tab lands on a signed-in URL, the extension reads that provider's
   cookies (only under the four providers' domains), closes the tab and
   answers the page.
5. The app posts the cookies to `POST /api/providers/browser-signin/{target}`.
   Dravr refuses any cookie outside the provider's domain, proves the session
   with a read, and stores it exactly as a sciotte sign-in.

The page sends a keepalive every 20 s, so a sign-in that takes minutes
outlives Chrome's service-worker idle timer.

## Load it (until it is on the stores)

**Chrome, Edge, Brave, Opera, Arc**: open `chrome://extensions` (`edge://extensions`
on Edge), switch on **Developer mode**, click **Load unpacked** and pick this
folder. Chrome lists `background.scripts` and `browser_specific_settings` as
unrecognised keys and ignores them: they are Firefox's.

**Firefox** (128 or later): open `about:debugging#/runtime/this-firefox`, click
**Load Temporary Add-on…** and pick this folder's `manifest.json`. A temporary
add-on stays loaded until Firefox restarts. When asked, allow it to access the
four providers' sites and app.dravr.ai. Firefox runs `background.scripts` and
ignores `service_worker`.

Once it is loaded, reload app.dravr.ai. A provider's sign-in window, for
example Garmin's **Ajouter l'historique**, then shows **Continuer avec Garmin**.

`npx web-ext lint --self-hosted --ignore-files "*.test.js"` validates the
manifest for Firefox. The one expected warning is that Firefox does not use
`service_worker`.

## Files

| File | What it is |
|---|---|
| `icons/` | The Dravr mark: the reduced Boreal Ripple (`frontend/public/favicon.svg`) at 16 and 32 px, the app icon (`frontend-mobile/assets/icon.png`) at 48 and 128 px |
| `manifest.json` | MV3 manifest: `cookies` and `tabs`, host permissions for the four providers only |
| `bridge.js` | The content script on app.dravr.ai: relays the page's window messages to a port into the extension |
| `background.js` | The background (a service worker in Chrome, an event page in Firefox): the port protocol, the sign-in tab, the cookie read |
| `signin.js` | Pure rules (accepted requests, signed-in detection, the cookie shape), tested without a browser |
| `*.test.js` | `bun test extensions/dravr-signin`: the rules, the background against a fake `chrome`, the relay against a fake window |
