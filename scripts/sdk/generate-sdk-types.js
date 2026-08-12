#!/usr/bin/env bun
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Auto-generates TypeScript type definitions from Pierre server tool schemas
// ABOUTME: Fetches MCP tool schemas as a global admin and converts them to TypeScript interfaces

const http = require('http');
const fs = require('fs');
const path = require('path');

/**
 * Configuration
 */
const SERVER_URL = process.env.PIERRE_SERVER_URL || 'http://localhost:8081';
const SERVER_PORT = process.env.HTTP_PORT || '8081';
// Output to shared mcp-types package (SDK re-exports from there). The script
// lives in scripts/sdk/, so the package is two levels up at <repo>/packages/.
const REPO_ROOT = path.join(__dirname, '../..');
const OUTPUT_DIR = path.join(REPO_ROOT, 'packages/mcp-types/src');
const OUTPUT_TOOLS_FILE = path.join(OUTPUT_DIR, 'tools.ts');
const OUTPUT_COMMON_FILE = path.join(OUTPUT_DIR, 'common.ts');
// Written by bin/setup-db-with-seeds-and-oauth-and-start-servers.sh, so a local
// dev stack needs no extra configuration to regenerate types.
const ADMIN_TOKEN_FILE = path.join(REPO_ROOT, 'logs/admin-token.txt');

/**
 * Issue an HTTP request against the local Pierre server.
 *
 * Resolves `{ statusCode, body }`; the caller decides what a non-200 means.
 */
function httpRequest({ method, requestPath, headers = {}, body }) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      { hostname: 'localhost', port: SERVER_PORT, path: requestPath, method, headers },
      (res) => {
        let data = '';
        res.on('data', (chunk) => {
          data += chunk;
        });
        res.on('end', () => resolve({ statusCode: res.statusCode, body: data }));
      }
    );

    req.on('error', (err) => {
      reject(new Error(`Failed to connect to server: ${err.message}`));
    });

    if (body) {
      req.write(body);
    }
    req.end();
  });
}

/**
 * Exchange admin credentials for a JWT via the OAuth2 password grant.
 */
async function loginAsAdmin(email, password) {
  const form = new URLSearchParams({
    grant_type: 'password',
    username: email,
    password
  }).toString();

  const { statusCode, body } = await httpRequest({
    method: 'POST',
    requestPath: '/oauth/token',
    headers: {
      'Content-Type': 'application/x-www-form-urlencoded',
      'Content-Length': Buffer.byteLength(form)
    },
    body: form
  });

  if (statusCode !== 200) {
    throw new Error(`Admin login failed (${statusCode}): ${body}`);
  }

  const token = JSON.parse(body).access_token;
  if (!token) {
    throw new Error('Admin login response carried no access_token');
  }
  return token;
}

/**
 * Resolve a bearer token for a global admin.
 *
 * `GET /mcp/tools` is tiered exactly like JSON-RPC `tools/list`, so only a
 * global admin (`User.is_admin`) sees the complete registry — which is what the
 * generated types must describe. Resolution order, most explicit first:
 *
 *   1. `PIERRE_ADMIN_TOKEN` — CI and any caller minting its own token.
 *   2. `ADMIN_EMAIL` + `ADMIN_PASSWORD` — a fresh password-grant login (both
 *      are exported by `.envrc` on a dev machine).
 *   3. `logs/admin-token.txt` — written by the dev-stack setup script.
 */
async function resolveAdminToken() {
  if (process.env.PIERRE_ADMIN_TOKEN) {
    console.log('🔑 Using PIERRE_ADMIN_TOKEN from the environment');
    return process.env.PIERRE_ADMIN_TOKEN;
  }

  const email = process.env.ADMIN_EMAIL;
  const password = process.env.ADMIN_PASSWORD;
  if (email && password) {
    console.log(`🔑 Logging in as ${email} for a fresh admin token`);
    return loginAsAdmin(email, password);
  }

  if (fs.existsSync(ADMIN_TOKEN_FILE)) {
    console.log(`🔑 Using the admin token at ${ADMIN_TOKEN_FILE}`);
    return fs.readFileSync(ADMIN_TOKEN_FILE, 'utf8').trim();
  }

  throw new Error(
    'No admin credentials available. Set PIERRE_ADMIN_TOKEN, or ADMIN_EMAIL + ' +
      `ADMIN_PASSWORD, or run the dev-stack setup script so ${ADMIN_TOKEN_FILE} exists.`
  );
}

/**
 * Fetch tool schemas from the Pierre server's discovery endpoint.
 *
 * `GET /mcp/tools` applies the same capability tiering as JSON-RPC
 * `tools/list`: an anonymous caller gets a 401, a tenant member gets their
 * filtered set, and a global admin gets every registered tool. Type generation
 * needs the complete catalog, so it authenticates as an admin.
 */
async function fetchToolSchemas(token) {
  const { statusCode, body } = await httpRequest({
    method: 'GET',
    requestPath: '/mcp/tools',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`
    }
  });

  if (statusCode === 401 || statusCode === 403) {
    throw new Error(
      `Discovery endpoint rejected the admin token (${statusCode}): ${body}. ` +
        'The token must belong to a user with the global is_admin flag.'
    );
  }
  if (statusCode !== 200) {
    throw new Error(`Server returned ${statusCode}: ${body}`);
  }

  try {
    return JSON.parse(body).tools || [];
  } catch (err) {
    throw new Error(`Failed to parse response: ${err.message}`);
  }
}

/**
 * Convert JSON schema property to TypeScript type
 */
function jsonSchemaToTypeScript(property, propertyName, required = false) {
  if (!property) {
    return 'any';
  }

  const isOptional = !required;
  const optionalMarker = isOptional ? '?' : '';

  // Handle type arrays (e.g., ["string", "null"])
  if (Array.isArray(property.type)) {
    const types = property.type
      .filter(t => t !== 'null')
      .map(t => jsonSchemaToTypeScript({ type: t }, propertyName, true));
    const typeStr = types.length > 1 ? types.join(' | ') : types[0];
    return property.type.includes('null') ? `${typeStr} | null` : typeStr;
  }

  switch (property.type) {
    case 'string':
      if (property.enum) {
        return property.enum.map(e => `"${e}"`).join(' | ');
      }
      return 'string';
    case 'number':
    case 'integer':
      return 'number';
    case 'boolean':
      return 'boolean';
    case 'array':
      if (property.items) {
        const itemType = jsonSchemaToTypeScript(property.items, propertyName, true);
        return `${itemType}[]`;
      }
      return 'any[]';
    case 'object':
      if (property.properties) {
        return generateInterfaceFromProperties(property.properties, property.required || []);
      }
      if (property.additionalProperties) {
        const valueType = jsonSchemaToTypeScript(property.additionalProperties, propertyName, true);
        return `Record<string, ${valueType}>`;
      }
      return 'Record<string, any>';
    case 'null':
      return 'null';
    default:
      return 'any';
  }
}

/**
 * Generate inline interface from properties
 */
function generateInterfaceFromProperties(properties, requiredFields = []) {
  if (!properties || Object.keys(properties).length === 0) {
    return '{}';
  }

  // Sort properties alphabetically for deterministic output
  const sortedEntries = Object.entries(properties).sort(([a], [b]) => a.localeCompare(b));
  const fields = sortedEntries.map(([name, prop]) => {
    const isRequired = requiredFields.includes(name);
    const tsType = jsonSchemaToTypeScript(prop, name, isRequired);
    const optional = isRequired ? '' : '?';
    const description = prop.description ? `\n  /** ${prop.description} */` : '';
    return `${description}\n  ${name}${optional}: ${tsType};`;
  });

  return `{\n${fields.join('\n')}\n}`;
}

/**
 * Convert tool name to PascalCase for interface names
 */
function toPascalCase(str) {
  return str
    .split('_')
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join('');
}

/**
 * Generate TypeScript tool types from tool schemas
 */
function generateToolsTypeScript(tools) {
  // Sort tools alphabetically by name for deterministic output
  const sortedTools = [...tools].sort((a, b) => a.name.localeCompare(b.name));

  const header = `// ABOUTME: Auto-generated TypeScript type definitions for Pierre MCP tool parameters
// ABOUTME: Generated from server tool schemas - DO NOT EDIT MANUALLY
//
// Tool count: ${sortedTools.length}
// To regenerate: bun run generate (from packages/mcp-types)

/* eslint-disable @typescript-eslint/no-explicit-any */

// ============================================================================
// TOOL PARAMETER TYPES
// ============================================================================

// Note: connect_to_dravr removed - SDK bridge handles authentication locally via RFC 8414 discovery

`;

  const paramTypes = sortedTools.map(tool => {
    const interfaceName = `${toPascalCase(tool.name)}Params`;
    const description = tool.description ? `\n/**\n * ${tool.description}\n */` : '';

    if (!tool.inputSchema || !tool.inputSchema.properties || Object.keys(tool.inputSchema.properties).length === 0) {
      return `${description}\nexport interface ${interfaceName} {}\n`;
    }

    const properties = tool.inputSchema.properties;
    const required = tool.inputSchema.required || [];

    // Sort properties alphabetically for deterministic output
    const sortedProps = Object.entries(properties).sort(([a], [b]) => a.localeCompare(b));
    const fields = sortedProps.map(([name, prop]) => {
      const isRequired = required.includes(name);
      const tsType = jsonSchemaToTypeScript(prop, name, isRequired);
      const optional = isRequired ? '' : '?';
      const propDescription = prop.description ? `\n  /** ${prop.description} */` : '';
      return `${propDescription}\n  ${name}${optional}: ${tsType};`;
    });

    return `${description}\nexport interface ${interfaceName} {\n${fields.join('\n')}\n}\n`;
  }).join('\n');

  const responseTypesHeader = `
// ============================================================================
// TOOL RESPONSE TYPES
// ============================================================================

/**
 * Generic MCP tool response wrapper
 */
export interface McpToolResponse {
  content?: Array<{
    type: string;
    text?: string;
    [key: string]: any;
  }>;
  isError?: boolean;
  [key: string]: any;
}

/**
 * MCP error response
 */
export interface McpErrorResponse {
  code: number;
  message: string;
  data?: any;
}

`;

  // Generate a union type of all tool names (already sorted)
  const toolNamesUnion = `
// ============================================================================
// TOOL NAME TYPES
// ============================================================================

/**
 * Union type of all available tool names
 */
export type ToolName = ${sortedTools.map(t => `"${t.name}"`).join(' | ')};

/**
 * Map of tool names to their parameter types
 */
export interface ToolParamsMap {
${sortedTools.map(t => `  "${t.name}": ${toPascalCase(t.name)}Params;`).join('\n')}
}
`;

  return header + paramTypes + responseTypesHeader + toolNamesUnion;
}

/**
 * Generate common data types (Activity, Athlete, etc.)
 */
function generateCommonTypeScript() {
  return `// ABOUTME: Common data types for Pierre MCP tools (Activity, Athlete, Stats, etc.)
// ABOUTME: Generated from server tool schemas - DO NOT EDIT MANUALLY

/* eslint-disable @typescript-eslint/no-explicit-any */

// ============================================================================
// COMMON DATA TYPES
// ============================================================================

/**
 * Fitness activity data structure
 */
export interface Activity {
  id: string;
  name: string;
  type: string;
  distance?: number;
  duration?: number;
  moving_time?: number;
  elapsed_time?: number;
  total_elevation_gain?: number;
  start_date?: string;
  start_date_local?: string;
  timezone?: string;
  average_speed?: number;
  max_speed?: number;
  average_cadence?: number;
  average_heartrate?: number;
  max_heartrate?: number;
  average_watts?: number;
  kilojoules?: number;
  device_watts?: boolean;
  has_heartrate?: boolean;
  calories?: number;
  description?: string;
  trainer?: boolean;
  commute?: boolean;
  manual?: boolean;
  private?: boolean;
  visibility?: string;
  flagged?: boolean;
  gear_id?: string;
  from_accepted_tag?: boolean;
  upload_id?: number;
  external_id?: string;
  achievement_count?: number;
  kudos_count?: number;
  comment_count?: number;
  athlete_count?: number;
  photo_count?: number;
  map?: {
    id?: string;
    summary_polyline?: string;
    polyline?: string;
  };
  [key: string]: any;
}

/**
 * Athlete profile data structure
 */
export interface Athlete {
  id: string;
  username?: string;
  resource_state?: number;
  firstname?: string;
  lastname?: string;
  bio?: string;
  city?: string;
  state?: string;
  country?: string;
  sex?: string;
  premium?: boolean;
  summit?: boolean;
  created_at?: string;
  updated_at?: string;
  badge_type_id?: number;
  weight?: number;
  profile_medium?: string;
  profile?: string;
  friend?: any;
  follower?: any;
  ftp?: number;
  [key: string]: any;
}

/**
 * Athlete statistics data structure
 */
export interface Stats {
  biggest_ride_distance?: number;
  biggest_climb_elevation_gain?: number;
  recent_ride_totals?: ActivityTotals;
  recent_run_totals?: ActivityTotals;
  recent_swim_totals?: ActivityTotals;
  ytd_ride_totals?: ActivityTotals;
  ytd_run_totals?: ActivityTotals;
  ytd_swim_totals?: ActivityTotals;
  all_ride_totals?: ActivityTotals;
  all_run_totals?: ActivityTotals;
  all_swim_totals?: ActivityTotals;
  [key: string]: any;
}

/**
 * Activity totals for statistics
 */
export interface ActivityTotals {
  count?: number;
  distance?: number;
  moving_time?: number;
  elapsed_time?: number;
  elevation_gain?: number;
  achievement_count?: number;
}

/**
 * Fitness configuration profile
 */
export interface FitnessConfig {
  athlete_info?: {
    age?: number;
    weight?: number;
    height?: number;
    sex?: string;
    ftp?: number;
    max_heart_rate?: number;
    resting_heart_rate?: number;
    vo2_max?: number;
  };
  training_zones?: {
    heart_rate?: Zone[];
    power?: Zone[];
    pace?: Zone[];
  };
  goals?: Goal[];
  preferences?: {
    distance_unit?: string;
    weight_unit?: string;
    [key: string]: any;
  };
  [key: string]: any;
}

/**
 * Training zone definition
 */
export interface Zone {
  zone: number;
  name: string;
  min: number;
  max: number;
  description?: string;
}

/**
 * Fitness goal definition
 */
export interface Goal {
  id?: string;
  type: string;
  target_value: number;
  target_date: string;
  activity_type?: string;
  description?: string;
  progress?: number;
  status?: string;
  created_at?: string;
  updated_at?: string;
}

/**
 * Provider connection status
 */
export interface ConnectionStatus {
  provider: string;
  connected: boolean;
  last_sync?: string;
  expires_at?: string;
  scopes?: string[];
  [key: string]: any;
}

/**
 * Notification data structure
 */
export interface Notification {
  id: string;
  type: string;
  message: string;
  provider?: string;
  success?: boolean;
  created_at: string;
  read: boolean;
  [key: string]: any;
}
`;
}

/**
 * Main execution
 */
async function main() {
  console.log('🔧 Pierre MCP Types Generator');
  console.log('==============================\n');

  try {
    const token = await resolveAdminToken();

    console.log(`📡 Fetching tool schemas from ${SERVER_URL}:${SERVER_PORT}/mcp/tools...`);

    const tools = await fetchToolSchemas(token);
    console.log(`✅ Fetched ${tools.length} tool schemas\n`);

    // Ensure output directory exists
    if (!fs.existsSync(OUTPUT_DIR)) {
      fs.mkdirSync(OUTPUT_DIR, { recursive: true });
    }

    console.log('🔨 Generating TypeScript definitions...');

    // Generate and write tools.ts
    const toolsTs = generateToolsTypeScript(tools);
    console.log(`💾 Writing to ${OUTPUT_TOOLS_FILE}...`);
    fs.writeFileSync(OUTPUT_TOOLS_FILE, toolsTs, 'utf8');

    // Generate and write common.ts
    const commonTs = generateCommonTypeScript();
    console.log(`💾 Writing to ${OUTPUT_COMMON_FILE}...`);
    fs.writeFileSync(OUTPUT_COMMON_FILE, commonTs, 'utf8');

    console.log(`\n✅ Successfully generated types for ${tools.length} tools!\n`);
    console.log('📋 Generated files:');
    console.log(`   - ${OUTPUT_TOOLS_FILE} (${tools.length} tool parameter interfaces)`);
    console.log(`   - ${OUTPUT_COMMON_FILE} (common data types)`);

    console.log('\n✨ Type generation complete!');
    console.log(`\n💡 Import types in your code:`);
    console.log(`   import { GetActivitiesParams, Activity } from '@pierre/mcp-types';\n`);

  } catch (error) {
    console.error('❌ Error generating types:', error.message);
    console.error('\n🔍 Troubleshooting:');
    console.error('   1. Ensure Pierre server is running on port', SERVER_PORT);
    console.error('   2. Verify the discovery endpoint is reachable at', `${SERVER_URL}:${SERVER_PORT}/mcp/tools`);
    console.error('   3. The endpoint is admin-gated: supply PIERRE_ADMIN_TOKEN, or');
    console.error('      ADMIN_EMAIL + ADMIN_PASSWORD, or run the dev-stack setup script');
    console.error('      so logs/admin-token.txt holds a fresh global-admin JWT');
    console.error('\n💡 Start the full dev stack with: ./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh');
    process.exit(1);
  }
}

// Run if called directly
if (require.main === module) {
  main();
}

module.exports = {
  fetchToolSchemas,
  generateToolsTypeScript,
  generateCommonTypeScript,
  resolveAdminToken,
  loginAsAdmin
};
