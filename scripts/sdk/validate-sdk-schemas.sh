#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# ABOUTME: Validates SDK response schemas against server tool definitions
# ABOUTME: Detects schema drift by comparing tool counts and running schema tests

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "========================================"
echo "SDK Schema Validation"
echo "========================================"

# Check that we're in the right directory
if [ ! -f "$PROJECT_ROOT/sdk/package.json" ]; then
    echo "Error: SDK not found at $PROJECT_ROOT/sdk"
    exit 1
fi

cd "$PROJECT_ROOT/sdk"

# Step 1: Verify response-schemas.ts exists
echo ""
echo "Step 1: Checking response-schemas.ts exists..."
if [ ! -f "src/response-schemas.ts" ]; then
    echo "Error: src/response-schemas.ts not found"
    echo "This file contains Zod schemas for validating tool responses"
    exit 1
fi
echo "OK: response-schemas.ts found"

# Step 2: Verify types.ts exists (input param types)
echo ""
echo "Step 2: Checking types.ts exists..."
if [ ! -f "src/types.ts" ]; then
    echo "Error: src/types.ts not found"
    echo "Run 'bun run generate-types' to generate type definitions"
    exit 1
fi
echo "OK: types.ts found"

# Step 3: Count tool parameter interfaces (input schemas)
echo ""
echo "Step 3: Counting tool parameter interfaces..."
# src/types.ts is a barrel — `export * from '@pierre/mcp-types'` — and declares
# no interface of its own, so the count has to read the file the generator
# writes. A count taken against the barrel selects nothing and reports 0.
MCP_TYPES_SRC="$PROJECT_ROOT/packages/mcp-types/src/tools.ts"
if [ ! -f "$MCP_TYPES_SRC" ]; then
    echo "Error: $MCP_TYPES_SRC not found"
    echo "Regenerate with: cd packages/mcp-types && bun run generate"
    exit 1
fi
INPUT_TOOL_COUNT=$(grep -cE "^export interface [A-Za-z0-9_]+Params \{" "$MCP_TYPES_SRC" || true)
echo "Found $INPUT_TOOL_COUNT input parameter interfaces"

# Step 4: Count tools in response-schemas.ts (output schemas)
echo ""
echo "Step 4: Counting tools in response-schemas.ts..."
OUTPUT_TOOL_COUNT=$(grep -c "ResponseSchema = z.object" src/response-schemas.ts || true)
echo "Found $OUTPUT_TOOL_COUNT response schemas"

# Step 5: Count tools in ToolResponseSchemaMap
echo ""
echo "Step 5: Counting tools in ToolResponseSchemaMap..."
# The map's keys are bare identifiers, not quoted strings. Read them from inside
# the map body so a quoted key elsewhere in the file cannot inflate the count.
MAP_TOOL_COUNT=$(awk '/^export const ToolResponseSchemaMap/{in_map=1;next} in_map&&/^}/{exit} in_map' src/response-schemas.ts \
    | grep -cE "^[[:space:]]+[a-z][a-z0-9_]*:" || true)
echo "Found $MAP_TOOL_COUNT tools in ToolResponseSchemaMap"

# Step 6: Check coverage
echo ""
echo "Step 6: Validating coverage..."
# Every counter above selects from a file; a counter that reads 0 means the
# selection missed, not that the SDK is empty. An empty selection must never be
# green, so each zero is a hard failure and so is a map below the floor.
MIN_EXPECTED_TOOLS=35
COVERAGE_FAILED=false
if [ "$INPUT_TOOL_COUNT" -eq 0 ]; then
    echo "Error: no tool parameter interfaces found in $MCP_TYPES_SRC"
    COVERAGE_FAILED=true
fi
if [ "$OUTPUT_TOOL_COUNT" -eq 0 ]; then
    echo "Error: no response schemas found in src/response-schemas.ts"
    COVERAGE_FAILED=true
fi
if [ "$MAP_TOOL_COUNT" -lt "$MIN_EXPECTED_TOOLS" ]; then
    echo "Error: only $MAP_TOOL_COUNT tools in ToolResponseSchemaMap (expected >= $MIN_EXPECTED_TOOLS)"
    echo "Either tools lost their response schemas, or the map's shape changed and this count no longer sees it"
    COVERAGE_FAILED=true
fi
if [ "$COVERAGE_FAILED" = true ]; then
    exit 1
fi
echo "OK: coverage thresholds met"

# Step 7: TypeScript type check
echo ""
echo "Step 7: Running TypeScript type check..."
if ! bun run type-check 2>/dev/null; then
    echo "Error: TypeScript type check failed"
    echo "Fix type errors in SDK before continuing"
    exit 1
fi
echo "OK: TypeScript types are valid"

# Step 8: Run schema tests
echo ""
echo "Step 8: Running response schema tests..."
if ! bun run test test/unit/response-schemas.test.js 2>/dev/null; then
    echo "Error: Response schema tests failed"
    exit 1
fi
echo "OK: Response schema tests passed"

# Summary
echo ""
echo "========================================"
echo "Schema Validation Summary"
echo "========================================"
echo "Input param interfaces: $INPUT_TOOL_COUNT"
echo "Response schemas:       $OUTPUT_TOOL_COUNT"
echo "Tools in schema map:    $MAP_TOOL_COUNT"
echo ""
echo "All schema validation checks passed!"
