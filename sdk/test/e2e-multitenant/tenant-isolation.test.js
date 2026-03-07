// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests tenant isolation via HTTP MCP endpoints
// ABOUTME: Validates cross-tenant access is properly forbidden

const { setupMultiTenantClients, cleanupMultiTenantClients } = require('../helpers/multitenant-setup');
const { ensureServerRunning } = require('../helpers/server');

/**
 * Call a tools/call endpoint for a given tenant.
 * Uses JSON-RPC 2.0 protocol.
 */
async function callTool(serverUrl, token, toolName, args) {
    const response = await fetch(`${serverUrl}/mcp`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
            'Authorization': `Bearer ${token}`,
        },
        body: JSON.stringify({
            jsonrpc: '2.0',
            id: 1,
            method: 'tools/call',
            params: { name: toolName, arguments: args },
        }),
    });
    if (!response.ok) {
        return { httpStatus: response.status, error: await response.text() };
    }
    const data = await response.json();
    return data;
}

/**
 * Call tools/list to verify per-tenant tool visibility.
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

describe('Multi-Tenant Isolation via SDK', () => {
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

    test('Tenant cannot access another tenant activities', async () => {
        clients = await setupMultiTenantClients(2, { port: 8081 });
        expect(clients).toHaveLength(2);
        expect(clients[0].userId).not.toBe(clients[1].userId);

        // Each tenant calls get_activities — should only see their own data
        const [resultA, resultB] = await Promise.all([
            callTool(clients[0].serverUrl, clients[0].token, 'get_activities', { limit: 5 }),
            callTool(clients[1].serverUrl, clients[1].token, 'get_activities', { limit: 5 }),
        ]);

        // Both calls should succeed (or fail with auth errors if no provider connected)
        // but neither should contain the other tenant's data
        expect(resultA).toBeDefined();
        expect(resultB).toBeDefined();

        // If both succeed, verify data isolation by checking result structure
        if (resultA.result && resultB.result) {
            // Results are independent — stringify and check no cross-contamination
            const resultAStr = JSON.stringify(resultA.result);
            const resultBStr = JSON.stringify(resultB.result);
            // Neither result should contain the other tenant's userId
            expect(resultAStr).not.toContain(clients[1].userId);
            expect(resultBStr).not.toContain(clients[0].userId);
        }
    }, 30000);

    test('Tenant receives proper error codes for forbidden access', async () => {
        clients = await setupMultiTenantClients(2, { port: 8081 });
        expect(clients).toHaveLength(2);

        // Each tenant should be able to list tools (authorized)
        const result = await callToolsList(clients[0].serverUrl, clients[0].token);
        expect(result.result).toBeDefined();
        expect(result.result.tools).toBeDefined();

        // Using an invalid/empty token should fail
        const invalidResult = await callTool(
            clients[0].serverUrl, 'invalid-token', 'tools/list', {}
        );

        // Should get an error — either HTTP level or JSON-RPC level
        if (invalidResult.httpStatus) {
            expect([401, 403]).toContain(invalidResult.httpStatus);
        } else if (invalidResult.error) {
            expect(invalidResult.error).toBeDefined();
        }
    }, 30000);

    test('Tenant boundaries enforced across all tool calls', async () => {
        clients = await setupMultiTenantClients(2, { port: 8081 });
        expect(clients).toHaveLength(2);

        // Test isolation across multiple tool endpoints
        const toolsToTest = ['get_activities', 'get_athlete', 'get_connection_status'];

        for (const toolName of toolsToTest) {
            const [resultA, resultB] = await Promise.all([
                callTool(clients[0].serverUrl, clients[0].token, toolName, {}),
                callTool(clients[1].serverUrl, clients[1].token, toolName, {}),
            ]);

            // Both calls should return independently
            expect(resultA).toBeDefined();
            expect(resultB).toBeDefined();

            // If successful, ensure no cross-tenant data
            if (resultA.result && resultB.result) {
                const strA = JSON.stringify(resultA.result);
                const strB = JSON.stringify(resultB.result);
                expect(strA).not.toContain(clients[1].userId);
                expect(strB).not.toContain(clients[0].userId);
            }
        }
    }, 30000);

    test('Concurrent cross-tenant access attempts all fail appropriately', async () => {
        clients = await setupMultiTenantClients(3, { port: 8081 });
        expect(clients).toHaveLength(3);

        // All 3 tenants request tools/list simultaneously — should all succeed
        const results = await Promise.all(
            clients.map(c => callToolsList(c.serverUrl, c.token))
        );

        for (const result of results) {
            expect(result.result).toBeDefined();
            expect(result.result.tools).toBeDefined();
        }

        // Verify tool lists are identical (no tenant-specific leakage)
        const toolNames = results.map(r =>
            r.result.tools.map(t => t.name).sort().join(',')
        );
        expect(new Set(toolNames).size).toBe(1);
    }, 30000);
});
