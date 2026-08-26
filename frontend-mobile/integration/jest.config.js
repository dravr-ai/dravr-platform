// ABOUTME: Jest configuration for the mobile end-to-end suite: an `api` project and an `app` project.
// ABOUTME: `api` specs drive a live backend; `app` specs mount real screens over a stubbed HTTP transport.
//
// Run both (needs the Pierre server on port 8081):
//   bun run e2e:integration
// Run only the headless app-level specs (no backend, no simulator):
//   bun run e2e:integration --selectProjects app
//
// A project's global setup runs only when that project has tests to run, so
// `--selectProjects app` never reaches the `api` project's health check.

const path = require('path');

const mobileRoot = path.join(__dirname, '..');

// The app project renders real React Native screens, so it needs the exact
// transform, preset and module mapping the unit suite uses. Importing that
// config rather than restating it keeps the two from drifting — a new
// ESM-only dependency added to the allowlist there reaches these specs too.
const mobileJest = require(path.join(mobileRoot, 'jest.config.js'));

/**
 * Contract specs that talk to the real Pierre server on port 8081.
 *
 * Its global setup fails the run when no backend answers, which is the point:
 * a green `api` project means the endpoints the mobile client calls exist and
 * answer as the client expects.
 */
const apiProject = {
  displayName: 'api',

  // Run from the frontend-mobile directory
  rootDir: mobileRoot,

  // Only run tests in the integration/specs folder
  testMatch: ['<rootDir>/integration/specs/**/*.test.js'],

  // Global setup/teardown for server health checks and user creation
  globalSetup: path.join(__dirname, 'helpers/global-setup.js'),
  globalTeardown: path.join(__dirname, 'helpers/global-teardown.js'),

  // Use Node.js environment (no React Native runtime needed)
  testEnvironment: 'node',

  // Clear mocks between tests
  clearMocks: true,

  // Collect coverage from the integration test helpers
  collectCoverageFrom: ['<rootDir>/integration/helpers/**/*.js'],

  // Transform settings (none needed for plain JS)
  transform: {},

  // Don't transform node_modules
  transformIgnorePatterns: ['/node_modules/'],
};

/**
 * App-level specs: the real screens and hooks, the real `@pierre/api-client`,
 * and a stubbed axios adapter standing in for the network.
 *
 * Everything from the component tree down to the request URL is production
 * code, so these run headless — no simulator, no backend — while still
 * failing when the wire contract moves.
 */
const appProject = {
  ...mobileJest,
  displayName: 'app',
  rootDir: mobileRoot,
  moduleNameMapper: {
    // babel-preset-expo rewrites `process.env.X` into a read from
    // `expo/virtual/env`, which Metro supplies on device. The unit suite maps
    // every `expo/virtual/*` module to an empty object, which is fine while
    // nothing under test reads an env var — these specs import the real
    // `src/services/api`, which resolves its base URL that way. Map the env
    // module to the process environment so the client is configured the way
    // it is at runtime. Declared before the spread so it wins over the
    // catch-all pattern.
    '^expo/virtual/env$': path.join(__dirname, 'app/helpers/expoVirtualEnv.js'),
    ...mobileJest.moduleNameMapper,
  },
  testMatch: ['<rootDir>/integration/app/**/*.e2e.test.tsx'],
  // The unit suite excludes `/integration/`; this project is that folder.
  testPathIgnorePatterns: ['/node_modules/'],
};

/** @type {import('@jest/types').Config.InitialOptions} */
module.exports = {
  rootDir: mobileRoot,

  // Sequential execution - api tests share database state
  maxWorkers: 1,

  // Longer timeout: api specs call a real server, app specs mount whole screens.
  // Jest validates this only at the top level, and applies it to every project.
  testTimeout: 30000,

  // Verbose output for debugging
  verbose: true,

  projects: [apiProject, appProject],
};
