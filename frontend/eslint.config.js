// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: ESLint configuration for Pierre web frontend
// ABOUTME: Uses shared @pierre/eslint-config for consistent standards

import js from '@eslint/js';
import globals from 'globals';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import tseslint from 'typescript-eslint';
import { baseTypeScriptRules, baseReactRules, reactHooksRules } from '@pierre/eslint-config';

export default tseslint.config(
  { ignores: ['dist', 'coverage'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      // Shared rules from @pierre/eslint-config
      ...baseTypeScriptRules,
      ...baseReactRules,
      ...reactHooksRules,
      // Web-specific rules
      'react-refresh/only-export-components': [
        'warn',
        { allowConstantExport: true },
      ],
    },
  },
  {
    // Design system: form controls come from the ui/ primitives, never raw.
    // DESIGN.md §5 ships one editorial underline field; a hand-rolled control
    // silently re-introduces the boxed pre-Boreal language next to it.
    files: ['src/**/*.tsx'],
    ignores: ['src/components/ui/**'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector: 'JSXOpeningElement[name.name="textarea"]',
          message:
            'Use <Textarea> from components/ui — DESIGN.md §5 (editorial underline, no enclosing box).',
        },
        {
          selector: 'JSXOpeningElement[name.name="select"]',
          message:
            'Use <Select> from components/ui — DESIGN.md §5 (editorial underline, no enclosing box).',
        },
      ],
    },
  },
);
