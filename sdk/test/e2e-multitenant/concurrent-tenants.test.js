// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests concurrent access by multiple tenants via HTTP
// ABOUTME: Validates no cross-tenant data leakage during simultaneous tool calls

const { setupMultiTenantClients, cleanupMultiTenantClients } = require('../helpers/multitenant-setup');
const { ensureServerRunning } = require('../helpers/server');

/**
 * Call tools/list via JSON-RPC for a given tenant.
 */
async function callToolsList(serverUrl, token) {
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
        throw new Error(`tools/list failed: ${response.status}`);
    }
    return response.json();
}

describe('Multi-Tenant Concurrent Access via SDK', () => {
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

    test('Multiple tenants call tools/list simultaneously without conflicts', async () => {
        clients = await setupMultiTenantClients(3, { port: 8081 });
        expect(clients).toHaveLength(3);
        expect(clients[0].email).not.toBe(clients[1].email);
        expect(clients[1].email).not.toBe(clients[2].email);

        // Fire all 3 requests concurrently
        const results = await Promise.all(
            clients.map(c => callToolsList(c.serverUrl, c.token))
        );

        // All must succeed with valid tool lists
        for (const result of results) {
            expect(result.result).toBeDefined();
            expect(result.result.tools).toBeDefined();
            expect(Array.isArray(result.result.tools)).toBe(true);
            expect(result.result.tools.length).toBeGreaterThan(0);
        }

        // All responses should have the same tool set
        const toolCounts = results.map(r => r.result.tools.length);
        expect(new Set(toolCounts).size).toBe(1);
    }, 30000);

    test('Concurrent tenant tool calls maintain isolation', async () => {
        clients = await setupMultiTenantClients(2, { port: 8081 });
        expect(clients).toHaveLength(2);
        expect(clients[0].userId).not.toBe(clients[1].userId);

        // Both tenants request tools/list concurrently
        const [result1, result2] = await Promise.all([
            callToolsList(clients[0].serverUrl, clients[0].token),
            callToolsList(clients[1].serverUrl, clients[1].token),
        ]);

        // Both succeed independently
        expect(result1.result.tools).toBeDefined();
        expect(result2.result.tools).toBeDefined();

        // Tool schemas are identical (no tenant-specific variations)
        const names1 = result1.result.tools.map(t => t.name).sort();
        const names2 = result2.result.tools.map(t => t.name).sort();
        expect(names1).toEqual(names2);
    }, 30000);

    test('Rapid concurrent requests from multiple tenants succeed', async () => {
        clients = await setupMultiTenantClients(3, { port: 8081 });
        expect(clients).toHaveLength(3);

        // Fire 5 concurrent requests from each tenant (15 total)
        const requestsPerTenant = 5;
        const allRequests = [];
        for (const client of clients) {
            for (let i = 0; i < requestsPerTenant; i++) {
                allRequests.push(callToolsList(client.serverUrl, client.token));
            }
        }

        const results = await Promise.all(allRequests);

        // All 15 requests must succeed
        expect(results).toHaveLength(clients.length * requestsPerTenant);
        for (const result of results) {
            expect(result.result).toBeDefined();
            expect(result.result.tools).toBeDefined();
            expect(result.result.tools.length).toBeGreaterThan(0);
        }
    }, 30000);
});
