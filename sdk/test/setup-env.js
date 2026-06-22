// ABOUTME: Jest setup — disables the SDK's OAuth browser auto-open for the whole test run.
// ABOUTME: Sets PIERRE_DISABLE_BROWSER so in-process and spawned bridges never pop (or OOM) a browser.

// The bridge/session OAuth flow opens the system browser to /oauth2/login when it
// needs interactive auth. In a test run that is never completable and, run after run,
// accumulates tabs that can OOM the browser. Set the explicit kill switch here so every
// jest worker (and any bridge it spawns, which inherits this env) refuses the interactive
// flow / suppresses the popup. An explicit caller override is preserved.
if (process.env.PIERRE_DISABLE_BROWSER === undefined) {
  process.env.PIERRE_DISABLE_BROWSER = 'true';
}
