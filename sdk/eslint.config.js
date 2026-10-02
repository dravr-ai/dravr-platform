// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: ESLint configuration for the Pierre MCP SDK
// ABOUTME: Uses TypeScript ESLint with Node.js globals

import js from '@eslint/js'
import globals from 'globals'
import tseslint from 'typescript-eslint'

/**
 * No URL, and nothing else a server or a user supplies, may reach a shell.
 *
 * The bridge opens OAuth pages whose address comes from the server's discovery
 * document, so a `$(...)` in a URL fragment is a command the moment the URL
 * is interpolated into `exec("open ...")`. The launcher hands it to
 * `execFile` as one argv element. These rules keep the shell-string APIs out of
 * src/ altogether, rather than trusting each caller to quote: `exec` and
 * `execSync` take a command line, and `shell: true` turns the argv APIs back
 * into one. child_process is reached through named imports only, so the import
 * list is the whole of what a file can run.
 *
 * test/unit/shell-free-lint.test.js lints fixtures through this config to show
 * each rule fires.
 */
const SHELL_MESSAGE =
  'No shell in the SDK: pass the program and its arguments to execFile/spawn as an argv array, so nothing a server or user supplies is parsed as a command.';
const CHILD_PROCESS = '/^(node:)?child_process$/';

const shellStringImports = {
  paths: ['child_process', 'node:child_process'].map((name) => ({
    name,
    importNames: ['exec', 'execSync'],
    message: SHELL_MESSAGE,
  })),
};

const shellRestrictions = [
  {
    // Each of these holds the whole module, so `cp.exec` would get past the
    // named-import rule above (which already refuses `import * as cp`).
    selector: [
      `ImportDeclaration[source.value=${CHILD_PROCESS}] > ImportDefaultSpecifier`,
      `ImportExpression[source.value=${CHILD_PROCESS}]`,
      `CallExpression[callee.name='require'][arguments.0.value=${CHILD_PROCESS}]`,
      `TSExternalModuleReference[expression.value=${CHILD_PROCESS}]`,
    ].join(', '),
    message: `Import child_process by name. ${SHELL_MESSAGE}`,
  },
  {
    // Any value but a literal `false`: `shell: '/bin/sh'` and `shell: flag` are shells too.
    selector: ":matches(Property[key.name='shell'], Property[key.value='shell']):not([value.value=false])",
    message: SHELL_MESSAGE,
  },
];

export default tseslint.config(
  { ignores: ['dist', 'node_modules', 'coverage', '*.js', '*.mjs'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.ts'],
    languageOptions: {
      ecmaVersion: 2022,
      globals: {
        ...globals.node,
      },
    },
    rules: {
      '@typescript-eslint/no-unused-vars': ['warn', {
        argsIgnorePattern: '^_',
        varsIgnorePattern: '^_',
        caughtErrorsIgnorePattern: '^_',
      }],
      '@typescript-eslint/no-explicit-any': 'off',
      '@typescript-eslint/no-require-imports': 'off',
      '@typescript-eslint/no-empty-object-type': 'off',
      'no-console': 'off',
      'no-restricted-imports': ['error', shellStringImports],
      'no-restricted-syntax': ['error', ...shellRestrictions],
    },
  },
)
