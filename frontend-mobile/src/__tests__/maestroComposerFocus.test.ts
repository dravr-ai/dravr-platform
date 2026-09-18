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

describe('the composer focus helper', () => {
  const helper = read(HELPER);

  it('taps the composer until the hierarchy reports it focused', () => {
    // The tap is retried, so a first tap that lands before the field can take
    // IME focus is not the last word.
    expect(helper).toMatch(/- repeat:\s*\n\s+times: \d+\s*\n\s+while:\s*\n\s+notVisible:\s*\n\s+id: "message-input"\s*\n\s+focused: true/);
    expect(helper).toMatch(BARE_COMPOSER_TAP);
  });

  it('ends on an unconditional focus assertion, so a composer that never takes focus fails here', () => {
    // Without this, key codes typed by the next step would go to whatever holds
    // focus and the flow would fail some later step for no visible reason.
    const commands = helper.split('\n---\n')[1];
    const last = commands.trimEnd().split('\n').slice(-3).join('\n');
    expect(last).toMatch(/- assertVisible:\s*\n\s+id: "message-input"\s*\n\s+focused: true$/);
    expect(last).not.toContain('optional');
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
