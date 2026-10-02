// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Lints a source fixture through a project's real ESLint config and returns what its restriction rules said
// ABOUTME: Shared by the web (vitest), mobile (jest) and SDK (jest) suites, so each proves its own rules where its CI lane runs

const { execFile } = require('node:child_process');
const path = require('node:path');

/** The rules a "what may be written anywhere in the tree" restriction is expressed in. */
const RESTRICTION_RULES = new Set(['no-restricted-syntax', 'no-restricted-imports']);

/**
 * What the restriction rules of `target.cwd`'s own ESLint config report for
 * `source`, linted as though it were the file `target.file` of that project:
 * one `{ ruleId, message }` per hit.
 *
 * The rule id is part of the answer because one restriction is often written
 * as both rules with the same message (an import refused, and the identifier
 * refused under any import). A test that reads only messages stays green when
 * either rule is deleted, as long as the other still fires.
 *
 * The project's ESLint runs as its own process. Flat configs are ES modules,
 * and jest's module sandbox cannot `import()` one, so linting in-process works
 * under vitest only; a child process is the one way all three suites share.
 * CommonJS for the same reason: the SDK's jest loads it untransformed.
 *
 * Rejects when ESLint could not lint the fixture at all (a fixture that does
 * not parse, a config that does not load): either would otherwise read as
 * "no rule fired".
 */
function restrictionHits(target, source) {
  const eslintBin = path.join(
    path.dirname(require.resolve('eslint/package.json', { paths: [target.cwd] })),
    'bin',
    'eslint.js',
  );
  const args = [eslintBin, '--stdin', '--stdin-filename', path.join(target.cwd, target.file), '--format', 'json'];

  return new Promise((resolve, reject) => {
    const child = execFile(process.execPath, args, { cwd: target.cwd, maxBuffer: 16 * 1024 * 1024 }, (error, stdout, stderr) => {
      // Exit 1 is "linted, and found errors", which is the case under test.
      if (error && error.code !== 1) {
        reject(new Error(`ESLint could not lint ${target.file}: ${stderr || error.message}`));
        return;
      }
      let results;
      try {
        results = JSON.parse(stdout);
      } catch {
        reject(new Error(`ESLint printed no report for ${target.file}: ${stderr || stdout}`));
        return;
      }
      if (results.length !== 1) {
        // An ignored or unmatched file is reported as no result at all.
        reject(new Error(`ESLint's config for ${target.cwd} does not lint ${target.file}`));
        return;
      }
      const fatal = results[0].messages.filter((message) => message.fatal || message.ruleId === null);
      if (fatal.length > 0) {
        reject(new Error(`The fixture for ${target.file} was not linted: ${fatal.map((m) => m.message).join('; ')}`));
        return;
      }
      resolve(
        results[0].messages
          .filter((message) => RESTRICTION_RULES.has(message.ruleId))
          .map(({ ruleId, message }) => ({ ruleId, message })),
      );
    });
    child.stdin.end(source);
  });
}

module.exports = { restrictionHits };
