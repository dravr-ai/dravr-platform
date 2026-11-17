# Dravr sign-in extension

Signs the athlete in to Garmin, Strava, TrainingPeaks or COROS **in their own
browser** and hands that provider's session to Dravr (carnet#1617).

Since 2026-11-16, Garmin puts a reCAPTCHA in front of every sign-in that comes
from a cloud browser, sciotte's included, but still serves reads to a session
it signed in. So the sign-in runs where the provider sees a person: the
athlete's own Chrome. sciotte then reads with that session as after any sign-in.

## How it works

1. The app (`frontend/src/utils/signinExtension.ts`) opens a port to this
   extension. That is only possible from a page that `externally_connectable`
   admits: app.dravr.ai only. A local page could otherwise ask it for a
   provider's cookies the athlete is already signed in with.
2. The app asks Dravr for the provider's entry (`GET /api/providers/browser-signin`,
   served by sciotte from its provider configs). The entry holds the sign-in
   page, the URLs that say the athlete is signed in, and the cookie domain.
3. The extension opens the sign-in page in a real tab. The athlete signs in as
   usual, 2FA included.
4. When the tab lands on a signed-in URL, the extension reads that provider's
   cookies (`chrome.cookies`, only under the four providers' domains), closes
   the tab and answers the page.
5. The app posts the cookies to `POST /api/providers/browser-signin/{target}`.
   Dravr refuses any cookie outside the provider's domain, proves the session
   with a read, and stores it exactly as a sciotte sign-in.

The page sends a keepalive every 20 s, so a sign-in that takes minutes outlives
the service worker's idle timer.

## Side-load it (until it is on the Chrome Web Store)

1. Open `chrome://extensions` and switch on **Developer mode**.
2. Click **Load unpacked** and pick this folder (`extensions/dravr-signin`).
3. The extension shows the ID `hkcdkpfegfmnhlfialngmhfijbnpkkhf`. The `key` in
   `manifest.json` fixes it, and the app names that ID
   (`SIGNIN_EXTENSION_ID`).

Once it is loaded, the Garmin sign-in window on app.dravr.ai shows
**Continuer avec Garmin**.

## Files

| File | What it is |
|---|---|
| `manifest.json` | MV3 manifest: `cookies` and `tabs`, host permissions for the four providers only |
| `background.js` | The service worker: the port protocol, the sign-in tab, the cookie read |
| `signin.js` | Pure rules (accepted requests, signed-in detection, the cookie shape), tested without a browser |
| `signin.test.js` | `bun test extensions/dravr-signin` |
