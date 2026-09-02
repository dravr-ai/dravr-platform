// ABOUTME: Jest configuration for Pierre Mobile app testing
// ABOUTME: Uses jest-expo preset with React Native Testing Library

const path = require('path');

// Resolve React to a single instance to prevent dual-instance hooks crash
// in bun workspaces where react can exist in both local and root node_modules
const reactDir = path.dirname(require.resolve('react/package.json'));

module.exports = {
  preset: 'jest-expo',
  setupFilesAfterEnv: [
    '@testing-library/jest-native/extend-expect',
    '<rootDir>/jest.setup.js',
  ],
  transformIgnorePatterns: [
    // `uuid` v14 ships ESM-only; without this allowlist entry Jest
    // sees `export` syntax in `node_modules/uuid/dist/esm/*` and
    // fails to parse it.
    'node_modules/(?!((jest-)?react-native|@react-native(-community)?)|expo(nent)?|@expo(nent)?/.*|@expo-google-fonts/.*|@react-native-google-signin/.*|react-navigation|@react-navigation/.*|@unimodules/.*|unimodules|sentry-expo|native-base|react-native-svg|@pierre/.*|uuid)',
  ],
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx'],
  testMatch: ['**/__tests__/**/*.(ts|tsx|js)', '**/?(*.)+(spec|test).(ts|tsx|js)'],
  testPathIgnorePatterns: ['/node_modules/', '/e2e/', '/integration/'],
  collectCoverageFrom: [
    'src/**/*.{ts,tsx}',
    '!src/**/*.d.ts',
    '!src/**/index.ts',
  ],
  coverageThreshold: {
    global: {
      branches: 0,
      functions: 0,
      lines: 0,
      statements: 0,
    },
  },
  testEnvironment: 'node',
  moduleNameMapper: {
    '^@/(.*)$': '<rootDir>/src/$1',
    // The `react-native` export condition, spelled out: Metro resolves
    // `index-mobile.ts` on device, and the web entry re-exports an adapter
    // that uses `import.meta`, which Hermes cannot parse. Mapping jest at the
    // web entry made the test module graph diverge from the runtime one, so a
    // value import from this package failed to transform in tests while
    // working perfectly on device.
    '^@pierre/api-client$': '<rootDir>/../packages/api-client/src/index-mobile.ts',
    '^@pierre/chat-utils$': '<rootDir>/../packages/chat-utils/src/index.ts',
    '^@pierre/domain-utils$': '<rootDir>/../packages/domain-utils/src/index.ts',
    '^@pierre/shared-types$': '<rootDir>/../packages/shared-types/src/index.ts',
    '^@pierre/i18n$': '<rootDir>/../packages/i18n/src/index.ts',
    '^@pierre/i18n/native$': '<rootDir>/../packages/i18n/src/native.ts',
    // Strip the `.js` suffix off relative imports.
    //
    // The @pierre/* packages are authored for NodeNext ESM, where a relative
    // import must carry the *output* extension: `export { parseWorkoutPlan }
    // from './workout-plan.js'` in a .ts file. Vite resolves that back to the
    // .ts source; Jest takes it literally and fails with "Cannot find module
    // './workout-plan.js'". That made every @pierre package importing a
    // sibling untestable from here — which is why src/screens/chat had no
    // MessageList test until this mapping landed.
    //
    // Safe to apply globally: the rewritten specifier is resolved through
    // moduleFileExtensions, so a real .js file still resolves to itself. Only
    // relative paths match, so the bare-specifier mappings above and the react
    // pins below are untouched.
    '^(\\.{1,2}/.*)\\.js$': '$1',
    // babel-preset-expo rewrites every `process.env.EXPO_PUBLIC_*` read into
    // `require('expo/virtual/env').env.*`. Routing that to jest.setup.js — which
    // exports nothing — made the read throw "Cannot read properties of
    // undefined", so any module holding a build-time flag at module scope took
    // its whole suite down. Serve the real process.env for `env`; other virtual
    // modules keep the setup-file stand-in.
    '^expo/virtual/env$': '<rootDir>/jest.env.js',
    // NativeWind compiles `global.css` at build time; under jest the root
    // layout's stylesheet import is a plain module with nothing to export.
    // Without this stand-in the root layout cannot be rendered in a test.
    '\\.css$': '<rootDir>/jest.css.js',
    // Mock expo virtual modules for packages outside node_modules
    '^expo/virtual/(.*)$': '<rootDir>/jest.setup.js',
    // Ensure a single React instance across components and test renderer
    // Bun workspace hoisting can create duplicate react copies (local + root)
    '^react$': path.join(reactDir, 'index.js'),
    '^react/(.*)$': path.join(reactDir, '$1'),
  },
};
