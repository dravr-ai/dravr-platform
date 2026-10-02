// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: ESLint configuration for Pierre web frontend
// ABOUTME: Uses shared @pierre/eslint-config for consistent standards

import js from '@eslint/js';
import globals from 'globals';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import tseslint from 'typescript-eslint';
import {
  baseTypeScriptRules,
  baseReactRules,
  reactHooksRules,
  rawErrorMessageRestrictions,
} from '@pierre/eslint-config';

/**
 * `prose-invert` is Tailwind Typography's *dark* palette. Applied
 * unconditionally it wins in light mode too, painting `th` and headings white
 * on the Boreal cream canvas — the header row of a coach's table goes
 * invisible (observed 2026-08-13 on a five-column activity table).
 * `darkMode: 'class'` is configured, so `dark:prose-invert` is always the
 * correct spelling. A rule about how a class may be spelled anywhere in the
 * tree is a lint rule; src/__tests__/designRuleLint.test.ts shows it fires.
 */
const PROSE_INVERT_MESSAGE =
  'Write dark:prose-invert. Bare prose-invert applies the dark typography palette in light mode too, and table headers render white on the light canvas.';
const proseInvertRestrictions = [
  { selector: 'Literal[value=/(?<!dark:)\\bprose-invert\\b/]', message: PROSE_INVERT_MESSAGE },
  { selector: 'TemplateElement[value.raw=/(?<!dark:)\\bprose-invert\\b/]', message: PROSE_INVERT_MESSAGE },
];

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
    // A failed call is shown through describeApiError, never as the thrown
    // error's own message: that is axios's English, under any locale.
    files: ['src/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-syntax': ['error', ...rawErrorMessageRestrictions, ...proseInvertRestrictions],
    },
  },
  {
    // Design system: form controls come from the ui/ primitives, never raw.
    // DESIGN.md §5 ships one editorial underline field; a hand-rolled control
    // silently re-introduces the boxed pre-Boreal language next to it.
    files: ['src/**/*.tsx'],
    ignores: ['src/components/ui/**'],
    rules: {
      // One list per file, so the entries above are repeated here for the
      // files this block also matches.
      'no-restricted-syntax': [
        'error',
        ...rawErrorMessageRestrictions,
        ...proseInvertRestrictions,
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
