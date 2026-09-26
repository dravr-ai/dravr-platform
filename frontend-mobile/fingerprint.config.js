// ABOUTME: Narrows the inputs of the `fingerprint` runtime version to what changes the native binary
// ABOUTME: package.json scripts and eas.json are build tooling, so editing them must not strand OTA updates

// The runtime version (app.config.js, policy `fingerprint`) decides which
// installed binaries an `eas update` reaches: an update lands only on builds
// whose fingerprint equals the one computed from the checkout that publishes
// it. Every input below that is not native code is a way for two checkouts
// with the same native project to disagree, and the cost of a disagreement
// is an update that reaches no phone.
//
// - PackageJsonScriptsAll: the `scripts` block of package.json. Adding a
//   Maestro alias or a lint shortcut is not a native change, yet the default
//   skip only ignores the `android` and `ios` scripts.
// - eas.json: build-profile tooling (image, node and bun pins, channels,
//   environments). It was hashed by default, and the TestFlight lane once
//   rewrote it on the runner before building, so those binaries carried a
//   runtime version no commit could reproduce. The EXPO_PUBLIC_* values now
//   live in the EAS environments instead. app.config.js is still hashed as a
//   whole, so a change that reaches the native project still counts.
//
// __tests__/fingerprintConfig.test.ts fingerprints this project and fails
// if either input comes back, or if a native input goes missing.

/** @type {import('expo/fingerprint').Config} */
const config = {
  sourceSkips: ['PackageJsonScriptsAll'],
  ignorePaths: ['eas.json'],
};

module.exports = config;
