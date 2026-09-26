// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how the Maestro flows clear the two iOS overlays that hide or cover the app: "Save Password?" and Expo Go's tools button
// ABOUTME: Each helper must stay on every path that meets its overlay, and must act only when the overlay is what the screen shows

import { readdirSync, readFileSync, statSync } from 'fs';
import { join, relative } from 'path';

const MAESTRO_DIR = join(__dirname, '..', '..', '.maestro');
const WAIT_FOR_HOME = 'helpers/wait-for-home-after-login.yaml';
const HIDE_TOOLS_BUTTON = 'helpers/hide-dev-tools-button.yaml';

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

/** A flow's commands with comment and blank lines dropped, so order checks see commands only. */
function commandsOf(flow: string): string {
  return read(flow)
    .split('\n---\n')[1]
    .split('\n')
    .filter((line) => line.trim() !== '' && !line.trim().startsWith('#'))
    .join('\n');
}

/** Asserts that every needle occurs in `text`, each after the previous one. */
function expectInOrder(text: string, needles: string[]): void {
  let from = 0;
  for (const needle of needles) {
    const at = text.indexOf(needle, from);
    expect({ needle, found: at >= 0 }).toEqual({ needle, found: true });
    from = at + needle.length;
  }
}

describe('the wait for Home after a login', () => {
  const helper = commandsOf(WAIT_FOR_HOME);

  it('reads real flows, so an empty directory cannot pass as compliance', () => {
    expect(FLOWS.length).toBeGreaterThan(40);
    expect(FLOWS).toContain(WAIT_FOR_HOME);
    expect(FLOWS).toContain('helpers/login.yaml');
  });

  it('is how every flow that logs in and expects Home gets there', () => {
    // The sheet arrives seconds after the login screen has gone and hides the
    // tree the Home wait reads, so a flow waiting for Home on its own fails
    // behind it on any simulator where AutoFill is on.
    const loggingIn = FLOWS.filter((flow) => {
      const text = read(flow);
      return text.includes('id: "login-button"') && text.includes('id: "home-screen"');
    });
    expect(loggingIn).toEqual(expect.arrayContaining(['helpers/login.yaml', 'login/06-successful-login.yaml']));
    for (const flow of loggingIn) {
      expect({ flow, usesHelper: read(flow).includes('wait-for-home-after-login.yaml') }).toEqual({ flow, usesHelper: true });
    }
  });

  it('is the only flow that reaches the labelled-sheet dismissal', () => {
    const callers = FLOWS.filter((flow) => read(flow).includes('dismiss-save-password.yaml'));
    expect(callers).toEqual([WAIT_FOR_HOME]);
  });

  it('checks for the sheet on every round of the wait, not once', () => {
    expectInOrder(helper, [
      '- repeat:',
      'notVisible:',
      'id: "home-screen"',
      'visible: "Save Password?"',
      'file: dismiss-save-password.yaml',
      'id: "home-screen"',
      'optional: true',
    ]);
  });

  it('taps "Not Now" by position only on iOS and only when nothing of the app is exposed', () => {
    // Inside Expo Go the sheet is outside the hierarchy, so the tap cannot be
    // aimed by label; every condition below must hold before it fires.
    expectInOrder(helper, [
      'platform: iOS',
      'id: "home-screen"',
      'id: "login-screen"',
      'notVisible: "Accueil|Home|Discussion|Chat"',
      'point: "31%,62%"',
    ]);
    expect(helper.match(/point:/g)).toHaveLength(1);
  });

  it('ends on a required Home assertion', () => {
    const last = helper.split('\n').slice(-4).join('\n');
    expect(last).toMatch(/^- extendedWaitUntil:\s*\n\s+visible:\s*\n\s+id: "home-screen"\s*\n\s+timeout: \d+$/);
    expect(last).not.toContain('optional');
  });
});

describe("Expo Go's floating tools button", () => {
  const helper = commandsOf(HIDE_TOOLS_BUTTON);

  it('is switched off on every launch, once the login form is up', () => {
    expectInOrder(commandsOf('helpers/launch-app.yaml'), ['id: "email-input"', 'file: hide-dev-tools-button.yaml']);
  });

  it('is acted on only on the iOS login screen, where no tab bar shares its symbol', () => {
    // The app's settings tab draws the same gearshape.fill when selected.
    expectInOrder(helper, ['platform: iOS', 'id: "login-screen"', 'id: "gearshape.fill"', 'id: "gearshape.fill"']);
  });

  it('flips the Tools button switch itself, never the Fast refresh one above it', () => {
    // Maestro takes the first match in tree order, and Fast refresh's switch,
    // which reloads the app, is also right of the "Tools button" label.
    expectInOrder(helper, [
      'element: "Tools button"',
      'checked: true',
      'childOf:',
      'checked: true',
      'rightOf: "Tools button"',
      'below: "Fast refresh"',
      'id: "xmark"',
    ]);
  });

  it('proves the button gone before the flow goes on', () => {
    const last = helper.split('\n').slice(-4).join('\n');
    expect(last).toMatch(/- extendedWaitUntil:\s*\n\s+notVisible:\s*\n\s+id: "gearshape.fill"\s*\n\s+timeout: \d+$/);
  });
});
