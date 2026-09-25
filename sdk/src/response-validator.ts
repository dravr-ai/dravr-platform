// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Runtime response validation for MCP tool calls against each tool's advertised outputSchema
// ABOUTME: Compiles the outputSchemas from tools/list with ajv and checks every result's structuredContent

import { Ajv2020, type ErrorObject, type ValidateFunction } from "ajv/dist/2020.js";

/**
 * MCP tool response structure from the SDK
 */
interface McpToolResult {
  content?: Array<{
    type: string;
    text?: string;
    [key: string]: unknown;
  }>;
  isError?: boolean;
  structuredContent?: unknown;
  [key: string]: unknown;
}

/**
 * A tool as `tools/list` describes it — only the fields validation reads.
 */
export interface ToolWithOutputSchema {
  name: string;
  outputSchema?: Record<string, unknown>;
}

/**
 * Configuration for response validation behavior
 */
export interface ResponseValidatorConfig {
  /**
   * Whether validation is enabled (default: true in development, false in production)
   */
  enabled: boolean;

  /**
   * Whether to throw on validation errors (default: false - just log warnings)
   */
  strict: boolean;

  /**
   * Custom logger function (default: console.warn)
   */
  logger?: (message: string, details?: unknown) => void;

  /**
   * Whether to include raw response data in error logs (default: false for privacy)
   */
  logRawData: boolean;
}

const defaultConfig: ResponseValidatorConfig = {
  enabled: process.env.NODE_ENV !== "production",
  strict: false,
  logRawData: false,
};

let globalConfig: ResponseValidatorConfig = { ...defaultConfig };

/**
 * The server derives every outputSchema from the Rust type the tool answers
 * with (JSON Schema 2020-12, `$defs` for nested types), so the schema is the
 * contract and there is no second copy to keep in step with it. Formats such
 * as `uint32` or `double` are schemars annotations rather than string formats,
 * so format validation is off; structure, types and required keys are checked.
 */
const ajv = new Ajv2020({ allErrors: true, strict: false, validateFormats: false });

/** Compiled validators, keyed by tool name, from the latest `tools/list`. */
const validators = new Map<string, ValidateFunction>();

/**
 * Configure the response validator
 */
export function configureValidator(config: Partial<ResponseValidatorConfig>): void {
  globalConfig = { ...globalConfig, ...config };
}

/**
 * Get the current validator configuration
 */
export function getValidatorConfig(): Readonly<ResponseValidatorConfig> {
  return { ...globalConfig };
}

/**
 * Adopt the outputSchemas of a `tools/list` result, replacing the previous set.
 *
 * A schema that does not compile is logged and left out, so that tool's
 * results go unvalidated rather than every result failing.
 */
export function registerToolOutputSchemas(tools: ToolWithOutputSchema[]): void {
  const log = globalConfig.logger ?? console.warn;
  validators.clear();
  for (const tool of tools) {
    if (!tool.outputSchema) {
      continue;
    }
    try {
      validators.set(tool.name, ajv.compile(tool.outputSchema));
    } catch (error) {
      log(
        `[ResponseValidator] outputSchema of "${tool.name}" does not compile: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    }
  }
}

/**
 * Whether the latest `tools/list` gave `toolName` an outputSchema to validate against
 */
export function hasResponseSchema(toolName: string): boolean {
  return validators.has(toolName);
}

/**
 * Validation result with the original and validated data
 */
export interface ValidatedToolResult<T = unknown> {
  /** The original MCP result (unchanged) */
  result: McpToolResult;

  /** Whether validation passed */
  valid: boolean;

  /** The validated `structuredContent` (if valid) */
  data?: T;

  /** Validation errors (if invalid) */
  errors?: string[];

  /** Tool name that was called */
  toolName: string;
}

function describe(error: ErrorObject): string {
  return `${error.instancePath || "/"}: ${error.message ?? error.keyword}`;
}

/**
 * Validate an MCP tool result against the tool's advertised outputSchema.
 *
 * The spec (2025-11-25 server/tools, Output Schema) requires a server that
 * declares an outputSchema to return `structuredContent` conforming to it, so a
 * successful result without `structuredContent` is itself a failure. The
 * requirement makes no exception for error results, and the official SDK's
 * `Client.callTool` validates `structuredContent` on an `isError` result too,
 * so the server never sends one there: a refusal's machine-readable data (a
 * quota refusal's `retry_after_secs`, a Guardian block's `error_code`) arrives
 * as a JSON text block in `content`, after the message. An error result
 * therefore has no structured part to validate and is passed through, and a
 * tool with no outputSchema cannot be validated at all.
 *
 * @param toolName - The name of the tool that was called
 * @param result - The MCP tool result from callTool()
 * @returns ValidatedToolResult with validation status and the structured data
 */
export function validateMcpToolResponse<T = unknown>(
  toolName: string,
  result: McpToolResult
): ValidatedToolResult<T> {
  const log = globalConfig.logger ?? console.warn;
  const validate = validators.get(toolName);

  if (!globalConfig.enabled || !validate || result.isError) {
    return {
      result,
      valid: true,
      toolName,
    };
  }

  const errorMessages =
    result.structuredContent === undefined
      ? ["has an output schema but did not return structured content"]
      : validate(result.structuredContent)
        ? []
        : (validate.errors ?? []).map(describe);

  if (errorMessages.length === 0) {
    return {
      result,
      valid: true,
      data: result.structuredContent as T,
      toolName,
    };
  }

  const errorSummary = `[ResponseValidator] Tool "${toolName}" response validation failed:\n  - ${errorMessages.join("\n  - ")}`;

  if (globalConfig.strict) {
    throw new Error(errorSummary);
  }

  // Log warning (non-strict mode)
  log(errorSummary, globalConfig.logRawData ? result.structuredContent : undefined);

  return {
    result,
    valid: false,
    errors: errorMessages,
    toolName,
  };
}

/**
 * Create a validation wrapper for tool calls.
 * Use this to wrap callTool() for automatic validation.
 *
 * @example
 * ```typescript
 * const validatedCall = createValidatedToolCall(client.callTool.bind(client));
 * const result = await validatedCall("get_activities", { provider: "strava" });
 * if (result.valid) {
 *   console.log(result.data.activities);
 * }
 * ```
 */
export function createValidatedToolCall(
  callTool: (params: { name: string; arguments?: Record<string, unknown> }) => Promise<McpToolResult>
): <T = unknown>(
  toolName: string,
  args?: Record<string, unknown>
) => Promise<ValidatedToolResult<T>> {
  return async <T = unknown>(
    toolName: string,
    args?: Record<string, unknown>
  ): Promise<ValidatedToolResult<T>> => {
    const result = await callTool({ name: toolName, arguments: args });
    return validateMcpToolResponse<T>(toolName, result);
  };
}

/**
 * Type guard to check if validation passed
 */
export function isValidResponse<T>(
  result: ValidatedToolResult<T>
): result is ValidatedToolResult<T> & { valid: true; data: T } {
  return result.valid && result.data !== undefined;
}

/**
 * Statistics about validation results (for monitoring/debugging)
 */
export interface ValidationStats {
  totalCalls: number;
  validResponses: number;
  invalidResponses: number;
  skippedResponses: number;
  errorsByTool: Record<string, number>;
}

let stats: ValidationStats = {
  totalCalls: 0,
  validResponses: 0,
  invalidResponses: 0,
  skippedResponses: 0,
  errorsByTool: {},
};

/**
 * Get validation statistics
 */
export function getValidationStats(): Readonly<ValidationStats> {
  return { ...stats };
}

/**
 * Reset validation statistics
 */
export function resetValidationStats(): void {
  stats = {
    totalCalls: 0,
    validResponses: 0,
    invalidResponses: 0,
    skippedResponses: 0,
    errorsByTool: {},
  };
}

/**
 * Validate with statistics tracking
 */
export function validateWithStats<T = unknown>(
  toolName: string,
  result: McpToolResult
): ValidatedToolResult<T> {
  stats.totalCalls++;

  const validated = validateMcpToolResponse<T>(toolName, result);

  if (!globalConfig.enabled || !hasResponseSchema(toolName)) {
    stats.skippedResponses++;
  } else if (validated.valid) {
    stats.validResponses++;
  } else {
    stats.invalidResponses++;
    stats.errorsByTool[toolName] = (stats.errorsByTool[toolName] || 0) + 1;
  }

  return validated;
}
