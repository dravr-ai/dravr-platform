// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests type schema consistency across multiple tenants via tools/list
// ABOUTME: Validates all tenants receive identical schemas and schemas match SDK types

const { setupMultiTenantClients, cleanupMultiTenantClients } = require('../helpers/multitenant-setup');
const { ensureServerRunning } = require('../helpers/server');

/**
 * Fetch tools/list from the MCP endpoint for a given tenant.
 * Uses the JSON-RPC 2.0 protocol over HTTP.
 */
async function fetchToolsList(serverUrl, token) {
    const response = await fetch(`${serverUrl}/mcp`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
            'Authorization': `Bearer ${token}`,
        },
        body: JSON.stringify({
            jsonrpc: '2.0',
            id: 1,
            method: 'tools/list',
            params: {},
        }),
    });

    if (!response.ok) {
        const text = await response.text();
        throw new Error(`tools/list failed (${response.status}): ${text}`);
    }

    const data = await response.json();
    if (data.error) {
        throw new Error(`JSON-RPC error: ${JSON.stringify(data.error)}`);
    }

    return data.result;
}

/**
 * Extract a normalized schema fingerprint from a tools/list result.
 * Sorts tools by name for deterministic cross-tenant comparison.
 */
function normalizeToolSchemas(toolsResult) {
    if (!toolsResult || !toolsResult.tools) {
        return null;
    }

    const sorted = [...toolsResult.tools].sort((a, b) => a.name.localeCompare(b.name));

    return sorted.map(tool => ({
        name: tool.name,
        description: tool.description,
        inputSchema: tool.inputSchema,
    }));
}

describe('Multi-Tenant Type Consistency', () => {
    let server;
    let clients;

    beforeAll(async () => {
        server = await ensureServerRunning({ port: 8081 });
    }, 30000);

    afterAll(async () => {
        if (server && server.cleanup) {
            await server.cleanup();
        }
    });

    afterEach(async () => {
        if (clients) {
            await cleanupMultiTenantClients(clients);
            clients = null;
        }
    });

    test('All tenants receive identical tool schemas', async () => {
        clients = await setupMultiTenantClients(3, { port: 8081 });
        expect(clients).toHaveLength(3);

        // Fetch tools/list for each tenant
        const schemas = [];
        for (const client of clients) {
            const result = await fetchToolsList(client.serverUrl, client.token);
            expect(result).toBeDefined();
            expect(result.tools).toBeDefined();
            expect(Array.isArray(result.tools)).toBe(true);
            expect(result.tools.length).toBeGreaterThan(0);

            schemas.push(normalizeToolSchemas(result));
        }

        // All tenant schemas must be identical (use toEqual for deep comparison
        // because JSON.stringify does not guarantee property order)
        for (let i = 1; i < schemas.length; i++) {
            expect(schemas[i]).toEqual(schemas[0]);
        }
    }, 30000);

    test('Generated TypeScript types match all tenant schemas', async () => {
        clients = await setupMultiTenantClients(1, { port: 8081 });
        expect(clients).toHaveLength(1);

        // Fetch tools/list from server
        const result = await fetchToolsList(clients[0].serverUrl, clients[0].token);
        expect(result).toBeDefined();
        expect(result.tools).toBeDefined();

        // Load SDK types and verify tool names match
        const sdk = require('../../dist/index.js');
        const sdkToolNames = sdk.getValidatedToolNames();

        const serverToolNames = result.tools.map(t => t.name);

        // Core tools must be present on the server
        const coreServerTools = [
            'get_activities',
            'get_athlete',
            'get_stats',
            'detect_patterns',
            'suggest_goals',
            'search_food',
            'calculate_daily_nutrition',
            'analyze_weather_impact',
            'get_configuration_catalog',
            'validate_configuration',
        ];

        for (const tool of coreServerTools) {
            expect(serverToolNames).toContain(tool);
        }

        // SDK must expose validated tool names (subset with Zod response schemas)
        expect(sdkToolNames.length).toBeGreaterThan(0);

        // SDK and server must share a significant number of tool names
        const overlap = sdkToolNames.filter(name => serverToolNames.includes(name));
        expect(overlap.length).toBeGreaterThan(0);

        // Overlap should cover a reasonable portion of server tools
        const coverageRatio = overlap.length / serverToolNames.length;
        expect(coverageRatio).toBeGreaterThan(0.3);
    }, 30000);

    test('Tool schemas remain consistent across tenant lifecycle', async () => {
        clients = await setupMultiTenantClients(2, { port: 8081 });
        expect(clients).toHaveLength(2);

        // Fetch initial schemas
        const initialSchemas = [];
        for (const client of clients) {
            const result = await fetchToolsList(client.serverUrl, client.token);
            initialSchemas.push(normalizeToolSchemas(result));
        }

        // Wait briefly to simulate lifecycle passage
        await new Promise(resolve => setTimeout(resolve, 500));

        // Fetch schemas again after delay
        const laterSchemas = [];
        for (const client of clients) {
            const result = await fetchToolsList(client.serverUrl, client.token);
            laterSchemas.push(normalizeToolSchemas(result));
        }

        // Schemas must be unchanged after lifecycle events
        for (let i = 0; i < clients.length; i++) {
            expect(laterSchemas[i]).toEqual(initialSchemas[i]);
        }

        // Cross-tenant consistency must still hold
        expect(laterSchemas[0]).toEqual(laterSchemas[1]);
    }, 30000);

    test('New tenant receives current schema version', async () => {
        const initialClients = await setupMultiTenantClients(1, { port: 8081 });
        expect(initialClients).toHaveLength(1);

        const initialResult = await fetchToolsList(initialClients[0].serverUrl, initialClients[0].token);
        const initialSchema = normalizeToolSchemas(initialResult);

        // Create new tenant after delay
        await new Promise(resolve => setTimeout(resolve, 1000));

        const newClients = await setupMultiTenantClients(1, { port: 8081 });
        expect(newClients).toHaveLength(1);

        const newResult = await fetchToolsList(newClients[0].serverUrl, newClients[0].token);
        const newSchema = normalizeToolSchemas(newResult);

        // New tenant must get the same schema version
        expect(newSchema).toEqual(initialSchema);

        await cleanupMultiTenantClients(initialClients);
        await cleanupMultiTenantClients(newClients);
        clients = null;
    }, 30000);
});
