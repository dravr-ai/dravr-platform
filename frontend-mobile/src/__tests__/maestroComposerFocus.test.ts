// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that every Maestro flow reaches the chat composer through the focus helper, never by a bare tap
// ABOUTME: A tap that misses IME focus lets typed key codes reach Expo Go's dev menu, whose "r" reloads the app (carnet#355)

import { readdirSync, readFileSync, statSync } from 'fs';
import { join, relative } from 'path';

const MAESTRO_DIR = join(__dirname, '..', '..', '.maestro');
const HELPER = 'helpers/focus-message-input.yaml';

/** Every flow file under .maestro, as a path relative to that directory. */
function flowFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return flowFiles(path);
    return name.endsWith('.yaml') && name !== 'config.yaml' ? [relative(MAESTRO_DIR, path)] : [];
  });
}

const FLOWS = flowFiles(MAESTRO_DIR).sort();
const read = (flow: string): string => readFileSync(join(MAESTRO_DIR, flow), 'utf8');

/** A `tapOn:` whose target is the composer, in the two-line shape every flow writes it in. */
const BARE_COMPOSER_TAP = /tapOn:\s*\n\s+id:\s*"message-input"/;

/**
 * The helper's top-level commands, one string per `- ` item at column 0, with
 * comment and blank lines dropped so a branch ends on its last command.
 */
function topLevelCommands(flow: string): string[] {
  const commands = flow.split('\n---\n')[1];
  return commands
    .split(/\n(?=- )/)
    .map((item) =>
      item
        .split('\n')
        .filter((line) => line.trim() !== '' && !line.trim().startsWith('#'))
        .join('\n')
    )
    .filter((item) => item.startsWith('- '));
}

/** The last `count` lines of a command block. */
const tail = (block: string, count: number): string => block.split('\n').slice(-count).join('\n');

describe('the composer focus helper', () => {
  const helper = read(HELPER);
  const branches = topLevelCommands(helper);
  const branchFor = (platform: string): string =>
    branches.find((branch) => new RegExp(`platform: ${platform}\\n`).test(branch)) ?? '';

  it('taps the composer until the platform shows it focused', () => {
    // The tap is retried, so a first tap that lands before the field can take
    // IME focus is not the last word. Android reports focus on the field; the
    // iOS hierarchy reports every element unfocused, so there the software
    // keyboard's delete key coming up is the sign.
    expect(branchFor('Android')).toMatch(/- repeat:\s*\n\s+times: \d+\s*\n\s+while:\s*\n\s+notVisible:\s*\n\s+id: "message-input"\s*\n\s+focused: true/);
    expect(branchFor('iOS')).toMatch(/- repeat:\s*\n\s+times: \d+\s*\n\s+while:\s*\n\s+notVisible:\s*\n\s+id: "delete"/);
    expect(branchFor('Android')).toMatch(BARE_COMPOSER_TAP);
    expect(branchFor('iOS')).toMatch(BARE_COMPOSER_TAP);
  });

  it('has one branch per platform and nothing outside them', () => {
    // A top-level command would run on both platforms, and a platform with no
    // branch would type without any proof of focus.
    expect(branches).toHaveLength(2);
    expect(branchFor('Android')).not.toBe('');
    expect(branchFor('iOS')).not.toBe('');
  });

  it('ends each branch on a required focus assertion, so a composer that never takes focus fails here', () => {
    // Without this, text typed by the next step would go to whatever holds
    // focus and the flow would fail some later step for no visible reason.
    expect(tail(branchFor('Android'), 3)).toMatch(/- assertVisible:\s*\n\s+id: "message-input"\s*\n\s+focused: true$/);
    expect(tail(branchFor('iOS'), 2)).toMatch(/- assertVisible:\s*\n\s+id: "delete"$/);
    expect(tail(branchFor('Android'), 3)).not.toContain('optional');
    expect(tail(branchFor('iOS'), 2)).not.toContain('optional');
  });
});

describe('every flow that types into the composer', () => {
  it('reads real flows, so an empty directory cannot pass as compliance', () => {
    expect(FLOWS.length).toBeGreaterThan(40);
    expect(FLOWS).toContain(HELPER);
    expect(FLOWS).toContain('helpers/send-chat-message.yaml');
  });

  it('goes through the focus helper, directly or through send-chat-message', () => {
    const users = FLOWS.filter((flow) => flow !== HELPER && read(flow).includes('focus-message-input.yaml'));
    // The helper is reached from the sending helper and from the flows that
    // type without sending; both routes must exist for the contract to mean
    // anything.
    expect(users).toContain('helpers/send-chat-message.yaml');
    expect(users.length).toBeGreaterThanOrEqual(8);
  });

  it.each(FLOWS.filter((flow) => flow !== HELPER))('%s taps the composer only through the helper', (flow) => {
    expect(read(flow)).not.toMatch(BARE_COMPOSER_TAP);
  });
});
