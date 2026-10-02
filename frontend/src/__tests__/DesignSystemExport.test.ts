// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Design System artifact export — tokens drawn from the Boreal sources, one note per value, every card bundled
// ABOUTME: Runs the generator end to end and mounts its bundle with a card's own preview script, as the artifact page does

import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import React from 'react';
import * as ReactDOM from 'react-dom';
import * as ReactDOMClient from 'react-dom/client';
import { act } from 'react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import {
  BORDER_INK,
  BOREAL_DARK,
  BOREAL_LIGHT,
  CONTAINER_INKS_DARK,
  PRIMARY_HOVER,
  ghostBorder,
} from '@pierre/shared-constants';

import {
  CONTENT_ROOT,
  COVER,
  FRONTEND_ROOT,
  annotate,
  buildTokens,
  colorValues,
  componentSources,
  readNotes,
} from '../../scripts/design-system/sources';

/** The artifact's token-name grammar (tokens.json names). */
const TOKEN_NAME = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;
const COLOR_VALUE = /^(#[0-9a-f]{6}|rgba\(\d{1,3}, \d{1,3}, \d{1,3}, 0\.\d{2}\))$/;

const tokens = buildTokens({ repo: 'dravr-ai/dravr-platform', ref: 'test@0000000' }, componentSources(), '2026-01-01');
const colors = new Map(tokens.color.tokens.map((t) => [t.name, t.value]));
const styles = new Map(tokens.type.groups.flatMap((g) => g.styles.map((s) => [s.name, s])));

describe('design system tokens', () => {
  it('carries the Boreal tree, the bound inks and the hairlines with their source values in both schemes', () => {
    expect(tokens.color.tokens).toHaveLength(77);
    expect(colors.get('primary')).toEqual({ light: BOREAL_LIGHT.primary, dark: BOREAL_DARK.primary });
    expect(colors.get('surface-container-lowest')).toEqual({ light: '#ffffff', dark: '#0b0e0b' });
    expect(colors.get('primary-hover')).toEqual({ light: PRIMARY_HOVER.light, dark: PRIMARY_HOVER.dark });
    expect(colors.get('on-warning-container')).toEqual({ light: '#664c16', dark: CONTAINER_INKS_DARK.warning });
    expect(colors.get('ghost-border-faint')).toEqual({
      light: ghostBorder(BORDER_INK.light, 'faint'),
      dark: ghostBorder(BORDER_INK.dark, 'faint'),
    });
    expect(colors.get('mark-ink')).toEqual({ light: '#05331f', dark: '#a3d0be' });
    expect(colors.get('forest-900')).toBe('#00241a');
    expect(colors.get('provider-whoop')).toBe('#00d46a');
  });

  it('names every token once, in the artifact grammar, with a value the artifact can read', () => {
    const names = [
      ...tokens.color.tokens,
      ...tokens.spacing.tokens,
      ...tokens.radius.tokens,
      ...tokens.shadow.tokens,
      ...tokens.layout.tokens,
      ...tokens.opacity.tokens,
    ].map((t) => t.name);
    expect(new Set(names).size).toBe(names.length);
    for (const name of names) expect(name).toMatch(TOKEN_NAME);
    for (const t of tokens.color.tokens) {
      const values = typeof t.value === 'string' ? [t.value] : [t.value.light, t.value.dark];
      for (const v of values) expect(v, t.name).toMatch(COLOR_VALUE);
      expect(t.usage.length, t.name).toBeGreaterThan(10);
    }
  });

  it('takes the type scale from the Tailwind config and the phone ladder, the families from the font stacks', () => {
    expect(styles.get('text-sm')).toMatchObject({ fontSize: '13px', lineHeight: '18px', fontWeight: 500 });
    expect(styles.get('text-base')).toMatchObject({ fontSize: '15px', lineHeight: '23px' });
    expect(styles.get('text-xl')).toMatchObject({ fontSize: '18px', lineHeight: '24px', letterSpacing: '-0.01em' });
    expect(styles.get('wordmark')).toMatchObject({ fontSize: '18px', letterSpacing: '0.15em' });
    expect(styles.get('m-text-base')).toMatchObject({ fontSize: '16px', lineHeight: '22px' });
    expect(styles.get('m-text-3xl')).toMatchObject({ family: 'display', fontSize: '26px', lineHeight: '32px' });
    expect(tokens.type.families.display).toBe('"Schibsted Grotesk", "Plus Jakarta Sans", sans-serif');
    expect(tokens.type.families.mono.startsWith('"JetBrains Mono"')).toBe(true);
  });

  it('takes spacing, radii and the one shadow from the shared constants and the stylesheet', () => {
    const spacing = Object.fromEntries(tokens.spacing.tokens.map((t) => [t.name, t.value]));
    expect(spacing).toMatchObject({ 'space-xs': '4px', 'space-md': '16px', 'space-2xl': '48px', 'space-section': '136px' });
    const radius = Object.fromEntries(tokens.radius.tokens.map((t) => [t.name, t.value]));
    expect(radius).toEqual({
      'radius-sm': '2px', 'radius-md': '4px', 'radius-lg': '8px', 'radius-xl': '12px', 'radius-bubble': '14px', 'radius-full': '9999px',
    });
    expect(tokens.shadow.tokens[0].value).toEqual({
      light: '0 12px 24px -6px rgba(26, 28, 27, 0.12), 0 6px 12px -3px rgba(26, 28, 27, 0.08)',
      dark: '0 12px 24px -6px rgba(0, 0, 0, 0.55), 0 6px 12px -3px rgba(0, 0, 0, 0.45)',
    });
  });

  it('refuses a source value without a note and a note without a source value', () => {
    expect(() => annotate('color', [{ name: 'primary' }, { name: 'brand' }], { primary: 'The accent.' })).toThrow(
      'tokens without a note [brand]',
    );
    expect(() => annotate('color', [{ name: 'primary' }], { primary: 'The accent.', retired: 'Gone.' })).toThrow(
      'notes without a token [retired]',
    );
    expect(Object.keys(readNotes().colors)).toHaveLength(colorValues().length);
  });
});

// Why the reads below: the card folders (preview.html, README.md) are the
// export's content catalogue, and dist/project is what the generator writes.
// The generator is run for real and its output files are the result under
// test — read, parsed and, for the bundle, executed.
describe('design system cards', () => {
  const cards = readdirSync(path.join(CONTENT_ROOT, 'components')).filter((d) => d !== COVER);

  it('maps every card to the component source its bundle entry imports', () => {
    const sources = componentSources();
    expect(Object.keys(sources).sort()).toEqual([...cards].sort());
    expect(sources.Button).toBe('frontend/src/components/ui/Button.tsx');
    expect(sources.MessageBubble).toBe('frontend/src/components/chat/MessageBubble.tsx');
    for (const file of Object.values(sources)) expect(existsSync(path.join(FRONTEND_ROOT, '..', file)), file).toBe(true);
  });

  it('gives every card guidelines and a preview whose first line is the card marker', () => {
    for (const card of [...cards, COVER]) {
      const preview = readFileSync(path.join(CONTENT_ROOT, 'components', card, 'preview.html'), 'utf8');
      expect(preview.split('\n')[0], card).toMatch(/^<!-- @dsCard (group="[A-Za-z]+" )?height=\d+ -->$/);
    }
    for (const card of cards) {
      const readme = readFileSync(path.join(CONTENT_ROOT, 'components', card, 'README.md'), 'utf8');
      expect(readme.startsWith(`# ${card}\n`), card).toBe(true);
    }
  });
});

describe('design system generator', () => {
  const project = path.join(CONTENT_ROOT, 'dist', 'project');
  let bundle = '';

  beforeAll(() => {
    const run = spawnSync('bun', ['scripts/design-system/generate.ts'], { cwd: FRONTEND_ROOT, encoding: 'utf8' });
    expect(run.status, run.stderr).toBe(0);
    expect(run.stdout).toContain('project/components/bundle.js');
    bundle = readFileSync(path.join(project, 'components/bundle.js'), 'utf8');
  }, 120_000);

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('writes the index, the tokens and a bundle header listing every card', () => {
    const index = JSON.parse(readFileSync(path.join(project, 'design-system.json'), 'utf8'));
    expect(index).toMatchObject({ v: 3, layout: 'files', title: 'Dravr Boreal', namespace: 'Boreal' });
    expect(index.libraries).toEqual([{ name: 'react', version: '18' }, { name: 'react-dom', version: '18' }]);
    expect(Object.keys(index.assetGroups.Logos.files)).toHaveLength(7);
    const written = JSON.parse(readFileSync(path.join(project, 'tokens.json'), 'utf8'));
    expect(written.color.tokens).toHaveLength(77);
    const header = JSON.parse(/^\/\* @ds-bundle: (.*) \*\/\n/.exec(bundle)?.[1] ?? '{}');
    expect(header.namespace).toBe('Boreal');
    expect(header.components.map((c: { name: string }) => c.name)).toEqual([...readdirSync(path.join(CONTENT_ROOT, 'components'))].filter((d) => d !== COVER).sort());
    expect(bundle).not.toMatch(/<\/script|<!--/i);
  });

  it("ships the app's compiled sheet with dark driven by the artifact's data-theme", () => {
    const css = readFileSync(path.join(project, 'components/bundle.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    expect(css).toContain('.btn-primary');
    expect(css).toContain('.chat-bubble-user');
    expect(css).toContain(`[data-theme="dark"]{--color-primary:163 208 190`);
    expect(css).not.toMatch(/html\.dark/);
  });

  it('mounts the real components from the bundle with a card preview script', async () => {
    const win = window as unknown as Record<string, unknown>;
    win.React = React;
    win.ReactDOM = { ...ReactDOM, ...ReactDOMClient };
    new Function(bundle)();
    const preview = readFileSync(path.join(project, 'components/Button/preview.html'), 'utf8');
    const script = /<script>([\s\S]*)<\/script>/.exec(preview)?.[1] ?? '';
    document.body.innerHTML = '<div id="root"></div>';
    await act(async () => {
      new Function(script)();
    });
    const save = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Save plan');
    expect(save?.className).toContain('btn-primary');
    expect(document.querySelectorAll('button')).toHaveLength(9);
    // DravrLogo's /brand/ paths are rewritten to the artifact's uploaded marks.
    expect(bundle).toContain('/_blob/');
    expect(bundle).not.toContain('/brand/mark-');
  });
});
