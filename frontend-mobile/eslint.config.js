// ABOUTME: ESLint configuration for Pierre Mobile app (React Native/Expo)
// ABOUTME: Uses shared @pierre/eslint-config for consistent standards

import js from '@eslint/js';
import tsParser from '@typescript-eslint/parser';
import tsPlugin from '@typescript-eslint/eslint-plugin';
import reactPlugin from 'eslint-plugin-react';
import reactHooksPlugin from 'eslint-plugin-react-hooks';
// Import shared ESLint rules via relative path (mobile is outside npm workspaces)
import {
  baseTypeScriptRules,
  baseReactRules,
  reactHooksRules,
  rawErrorMessageRestrictions,
  testFileRules,
} from '../packages/eslint-config-pierre/index.js';

/**
 * D1 of Boreal v2.2: the tab bar is the platform's. A blur import, or the
 * glass container that hosted the old floating pill, means someone drew a bar
 * by hand again.
 */
const DRAWN_BAR_MESSAGE =
  "The tab bar is the platform's (Boreal v2.2 D1): no blur, no glass container, no hand-drawn bar.";

/**
 * React Native's core SafeAreaView is deprecated and warns on first access,
 * once per bundle load (carnet#356 counted 63 across two Android runs). The
 * importer then was react-native-css-interop 0.2.1, which NativeWind loads
 * eagerly; 0.2.7 registers react-native-safe-area-context's instead. This keeps
 * our own code from bringing the warning back.
 */
const SAFE_AREA_MESSAGE =
  "React Native's SafeAreaView is deprecated: import SafeAreaView from 'react-native-safe-area-context'.";

const restrictedImports = {
  paths: [
    { name: 'expo-blur', message: DRAWN_BAR_MESSAGE },
    { name: 'expo-glass-effect', message: DRAWN_BAR_MESSAGE },
    { name: 'react-native', importNames: ['SafeAreaView'], message: SAFE_AREA_MESSAGE },
  ],
  patterns: [{ group: ['**/ExpandableTabBar', '**/ExpandableTabBar.*'], message: DRAWN_BAR_MESSAGE }],
};

/**
 * Colours the appearance setting cannot move, and the chrome Boreal retired.
 *
 * A rule about what may be written anywhere in the tree is a lint rule: it
 * fires in the editor, on the line, and it parses the code instead of matching
 * it. __tests__/designRuleLint.test.ts lints fixtures through this config to
 * show each rule fires.
 *
 * - The drawn bar's components, by any name they are reached under.
 * - `#00241a` is the Boreal v1 primary, retired because it read as black at
 *   every size; it survived as a category colour and, with `#0d3b2e`, inside a
 *   login gradient.
 * - `gradients` was a module-level palette, so whatever drew from it rendered
 *   the same colours in light and dark. A third party's own brand gradient is
 *   legitimately fixed (DESIGN.md §2) and is not named `gradients`.
 */
const schemeAndChromeRestrictions = [
  {
    selector: ':matches(Identifier, JSXIdentifier)[name=/^(BlurView|GlassContainer|ExpandableTabBar)$/]',
    message: DRAWN_BAR_MESSAGE,
  },
  {
    selector: 'Literal[value=/#00241a|#0d3b2e/i]',
    message: 'The Boreal v1 primary is retired: it reads as black. Take the colour from useThemeColors().',
  },
  {
    selector: 'TemplateElement[value.raw=/#00241a|#0d3b2e/i]',
    message: 'The Boreal v1 primary is retired: it reads as black. Take the colour from useThemeColors().',
  },
  {
    selector: "JSXAttribute[name.name='colors'] MemberExpression[object.name='gradients']",
    message:
      'No gradient from a module-level palette: it cannot follow the appearance setting. Take the colours from useThemeColors().',
  },
];

// React Native / Browser globals
const rnGlobals = {
  console: 'readonly',
  process: 'readonly',
  __dirname: 'readonly',
  module: 'readonly',
  require: 'readonly',
  exports: 'readonly',
  setTimeout: 'readonly',
  clearTimeout: 'readonly',
  setInterval: 'readonly',
  clearInterval: 'readonly',
  fetch: 'readonly',
  FormData: 'readonly',
  URLSearchParams: 'readonly',
  URL: 'readonly',
  AbortController: 'readonly',
  AbortSignal: 'readonly',
  Headers: 'readonly',
  Request: 'readonly',
  Response: 'readonly',
  WebSocket: 'readonly',
  Blob: 'readonly',
  File: 'readonly',
  FileReader: 'readonly',
  alert: 'readonly',
  requestAnimationFrame: 'readonly',
  cancelAnimationFrame: 'readonly',
  __DEV__: 'readonly',
};

// Jest globals for test files
const jestGlobals = {
  jest: 'readonly',
  describe: 'readonly',
  it: 'readonly',
  test: 'readonly',
  expect: 'readonly',
  beforeEach: 'readonly',
  afterEach: 'readonly',
  beforeAll: 'readonly',
  afterAll: 'readonly',
  global: 'readonly',
  Event: 'readonly',
  MessageEvent: 'readonly',
  CloseEvent: 'readonly',
};

export default [
  js.configs.recommended,
  // Main source files
  {
    files: ['**/*.{ts,tsx}'],
    ignores: ['**/__tests__/**', '**/*.test.{ts,tsx}'],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: {
          jsx: true,
        },
      },
      globals: rnGlobals,
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      'react': reactPlugin,
      'react-hooks': reactHooksPlugin,
    },
    rules: {
      // Shared rules from @pierre/eslint-config
      ...baseTypeScriptRules,
      ...baseReactRules,
      ...reactHooksRules,
      // Mobile-specific rules
      'no-console': 'off',
      // A failed call is shown through describeApiError, never as the thrown
      // error's own message: that is axios's English, under any locale.
      'no-restricted-syntax': ['error', ...rawErrorMessageRestrictions, ...schemeAndChromeRestrictions],
      'no-restricted-imports': ['error', restrictedImports],
    },
    settings: {
      react: {
        version: 'detect',
      },
    },
  },
  // Test files - more relaxed rules
  {
    files: ['**/__tests__/**/*.{ts,tsx}', '**/*.test.{ts,tsx}'],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: {
          jsx: true,
        },
      },
      globals: {
        ...rnGlobals,
        ...jestGlobals,
      },
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      'react': reactPlugin,
      'react-hooks': reactHooksPlugin,
    },
    rules: {
      // Shared test file rules from @pierre/eslint-config
      ...testFileRules,
      'react-hooks/rules-of-hooks': 'error',
    },
    settings: {
      react: {
        version: 'detect',
      },
    },
  },
  // Global ignores
  {
    ignores: [
      'node_modules/',
      '.expo/',
      'app.config.js',
      'babel.config.js',
      'fingerprint.config.js',
      'metro.config.js',
      'jest.config.js',
      'jest.setup.js',
      'jest.expo-router.js',
      'jest.env.js',
      'jest.css.js',
      'react-native.config.js',
      'tailwind.config.js',
      '.detoxrc.js',
      'e2e/',
      'integration/',
      'eslint.config.js',
      'plugins/',
    ],
  },
];
