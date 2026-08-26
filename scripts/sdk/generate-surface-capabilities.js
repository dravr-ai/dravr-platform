#!/usr/bin/env node
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Generates the shared surface-capability catalogue from a running Pierre server
// ABOUTME: Same relationship generate-sdk-types.js has with the tool registry — server is the source

const http = require('http');
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const SERVER_PORT = process.env.HTTP_PORT || '8081';
const SERVER_HOST = process.env.PIERRE_SERVER_HOST || 'localhost';
const REPO_ROOT = path.join(__dirname, '../..');
const OUTPUT_FILE = path.join(
  REPO_ROOT,
  'packages/shared-constants/src/surface-capabilities.generated.ts'
);
const FINGERPRINT_SCRIPT = path.join(REPO_ROOT, 'scripts/ci/surface-capabilities-fingerprint.sh');

/**
 * Fetch the capability catalogue.
 *
 * The endpoint is unauthenticated — it serves compiled-in product capabilities,
 * identical for every caller — so unlike the tool-schema generator this needs no
 * credentials, only a server that is up.
 */
function fetchCatalogue() {
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: SERVER_HOST,
        port: SERVER_PORT,
        path: '/api/surfaces/capabilities',
        method: 'GET',
        headers: { Accept: 'application/json' }
      },
      (res) => {
        let data = '';
        res.on('data', (chunk) => {
          data += chunk;
        });
        res.on('end', () => {
          if (res.statusCode !== 200) {
            reject(new Error(`Server returned ${res.statusCode}: ${data}`));
            return;
          }
          try {
            resolve(JSON.parse(data));
          } catch (err) {
            reject(new Error(`Failed to parse response: ${err.message}`));
          }
        });
      }
    );
    req.on('error', (err) => reject(new Error(`Failed to connect to server: ${err.message}`)));
    req.end();
  });
}

/**
 * Read the capability digest the staleness gate recomputes.
 *
 * Shelling out to the shell script rather than reimplementing it here is the
 * point: two implementations of one digest disagree eventually, and the
 * disagreement reads as a stale file that regenerating does not fix.
 */
function capabilityDigest() {
  return execFileSync('bash', [FINGERPRINT_SCRIPT], { encoding: 'utf8' }).trim();
}

/** Quote a string as a TypeScript literal. */
function lit(value) {
  return `'${String(value).replace(/'/g, "\\'")}'`;
}

/** Render one surface's record entry. */
function surfaceEntry(surface) {
  const blocks = surface.blocks.map(lit).join(', ');
  return `  ${lit(surface.id)}: {
    id: ${lit(surface.id)},
    call_type: ${lit(surface.call_type)},
    prose: ${lit(surface.prose)},
    max_reply_chars: ${surface.max_reply_chars === null ? 'null' : surface.max_reply_chars},
    interactive: ${surface.interactive},
    progressive: ${lit(surface.progressive)},
    streams_text_deltas: ${surface.streams_text_deltas},
    max_tool_iterations: ${
      surface.max_tool_iterations === null ? 'null' : surface.max_tool_iterations
    },
    model_policy: ${lit(surface.model_policy)},
    blocks: [${blocks}],
  },`;
}

/** Render the whole generated module. */
function generate(catalogue, digest) {
  const { block_kinds: blockKinds, notification_screens: screens, surfaces } = catalogue;

  return `// ABOUTME: Auto-generated catalogue of what every chat surface renders, from the running server
// ABOUTME: Generated from GET /api/surfaces/capabilities - DO NOT EDIT MANUALLY
//
// Surfaces: ${surfaces.length} · Reply-block kinds: ${blockKinds.length} · Notification screens: ${screens.length}
// capability-digest: ${digest}
// content-digest: __CONTENT_DIGEST__
// To regenerate: bun run generate (from packages/shared-constants)

/**
 * Every reply-block kind the server can put in a turn envelope, in the order a
 * reply lays them out.
 *
 * A surface renders the kinds its row lists and is never handed another: the
 * pipeline reads the same capabilities this file was generated from before it
 * pushes a block.
 */
export const REPLY_BLOCK_KINDS = [
${blockKinds.map((kind) => `  ${lit(kind)},`).join('\n')}
] as const;

/** One renderable piece of an assistant reply, named as the wire names it. */
export type ReplyBlockKind = (typeof REPLY_BLOCK_KINDS)[number];

/** Every surface the platform serves a chat turn on. */
export const SURFACE_CAPABILITY_IDS = [
${surfaces.map((surface) => `  ${lit(surface.id)},`).join('\n')}
] as const;

/** A surface's telemetry id — the \`channel\` dimension on its pipeline spans. */
export type SurfaceCapabilityId = (typeof SURFACE_CAPABILITY_IDS)[number];

/** What one surface can put in front of an athlete. */
export interface SurfaceCapabilities {
  /** The surface's telemetry id. */
  id: SurfaceCapabilityId;
  /** The \`call_type\` stamped on this surface's LLM usage rows. */
  call_type: string;
  /** How the surface reads the natural-language part of a reply. */
  prose: 'markdown' | 'plain_text';
  /** Per-message character ceiling, or null where the transport imposes none. */
  max_reply_chars: number | null;
  /** Whether a rendered control's press reaches the platform. */
  interactive: boolean;
  /** Whether the transport can carry a reply before the turn finishes. */
  progressive: 'complete' | 'delta_channel';
  /** Whether a turn here streams partial text when the provider emits deltas. */
  streams_text_deltas: boolean;
  /** Fixed tool-loop budget, or null when coach/admin configuration resolves it. */
  max_tool_iterations: number | null;
  /** How the active model is resolved for a turn. */
  model_policy: 'use_stored' | 'override_with_env';
  /** Reply-block kinds this surface can be handed, in reply order. */
  blocks: readonly ReplyBlockKind[];
}

/** The \`SurfaceProfile::resolve\` table, one row per surface. */
export const SURFACE_CAPABILITIES: Record<SurfaceCapabilityId, SurfaceCapabilities> = {
${surfaces.map(surfaceEntry).join('\n')}
};

/**
 * The notification \`data.screen\` vocabulary, paired with the \`USER_SURFACES\`
 * id each token opens.
 *
 * The server declares this once; a client turns the surface id into its own
 * route through the registry rather than keeping a map of its own.
 */
export const NOTIFICATION_SCREEN_SURFACES = {
${screens.map((row) => `  ${lit(row.screen)}: ${lit(row.surface)},`).join('\n')}
} as const;

/** A screen name a notification's \`data.screen\` field can carry. */
export type NotificationScreen = keyof typeof NOTIFICATION_SCREEN_SURFACES;
`;
}

/** Fetch, render, write. */
async function main() {
  console.log('🔧 Pierre Surface Capability Catalogue Generator');
  console.log('================================================\n');

  try {
    console.log(
      `📡 Fetching the catalogue from http://${SERVER_HOST}:${SERVER_PORT}/api/surfaces/capabilities...`
    );
    const catalogue = await fetchCatalogue();
    console.log(
      `✅ ${catalogue.surfaces.length} surfaces, ${catalogue.block_kinds.length} block kinds, ` +
        `${catalogue.notification_screens.length} notification screens\n`
    );

    const digest = capabilityDigest();
    console.log(`🔒 Capability digest: ${digest}`);

    // Two digests, two directions. `capability-digest` covers the Rust
    // constructors, so a source change that nobody regenerated is caught.
    // `content-digest` covers the emitted rows, so a value edited by hand here
    // is caught -- which the source digest cannot see, because the messaging
    // ceilings are not literals in Rust at all: they come from canot's
    // ChannelDescriptor at runtime.
    const rendered = generate(catalogue, digest);
    fs.writeFileSync(OUTPUT_FILE, rendered, 'utf8');
    const contentDigest = execFileSync(
      'bash',
      [FINGERPRINT_SCRIPT, '--content'],
      { encoding: 'utf8' }
    ).trim();
    fs.writeFileSync(
      OUTPUT_FILE,
      rendered.replace('__CONTENT_DIGEST__', contentDigest),
      'utf8'
    );
    console.log(`🔒 Content digest: ${contentDigest}`);
    console.log(`💾 Wrote ${OUTPUT_FILE}\n`);
    console.log('✨ Catalogue generation complete!');
  } catch (error) {
    console.error('❌ Error generating the catalogue:', error.message);
    console.error('\n🔍 Troubleshooting:');
    console.error(`   1. Ensure a Pierre server is running on port ${SERVER_PORT}`);
    console.error('   2. The endpoint needs no credentials, only a reachable server');
    console.error('   3. Start the dev stack with: ./bin/start-server.sh');
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = { generate, fetchCatalogue };
