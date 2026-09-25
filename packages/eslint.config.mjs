// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one ESLint configuration every shared package lints under, built from @pierre/eslint-config
// ABOUTME: A package's `eslint src/` finds it by walking up, so no package carries a copy of its own

import js from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import tseslint from 'typescript-eslint';
import {
  baseTypeScriptRules,
  rawErrorMessageRestrictions,
  reactHooksRules,
} from '@pierre/eslint-config';

/**
 * The shared packages are the code both clients compile, so they lint under
 * the web client's TypeScript rules and the same refusal of raw error text.
 * No browser or React Native globals: a package that reaches for `window` or
 * a native module is no longer shared.
 *
 * ESLint 9 looks for its configuration from the working directory upwards,
 * and every package's `lint` script runs from the package itself, so this file
 * is the one each of them finds.
 */
export default tseslint.config(
  { ignores: ['**/node_modules/**', '**/dist/**', 'eslint-config-pierre/**', 'tsconfig-pierre/**'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      // typescript-eslint otherwise infers the root from every config it has
      // seen in the process, and refuses to guess once there are several.
      parserOptions: { tsconfigRootDir: import.meta.dirname },
    },
    plugins: {
      'react-hooks': reactHooks,
    },
    rules: {
      ...baseTypeScriptRules,
      ...reactHooksRules,
      'no-restricted-syntax': ['error', ...rawErrorMessageRestrictions],
    },
  },
);
