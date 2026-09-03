// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Stateless MCP client over Streamable HTTP for protocol revision 2026-07-28
// ABOUTME: Sends per-request _meta and routing headers, reads JSON or SSE replies, polls Tasks

/**
 * The protocol revision this client speaks to the Dravr server.
 *
 * Revision 2026-07-28 has no `initialize` handshake and no session: every request
 * carries its protocol version and the client's capabilities in `params._meta`, the
 * server answers each request on its own, and a result names its shape in `resultType`.
 * `server/discover` replaces the capability exchange the handshake used to do.
 */
export const MCP_PROTOCOL_VERSION = "2026-07-28";

/** The Tasks extension identifier, declared per request and advertised by the server. */
export const TASKS_EXTENSION_ID = "io.modelcontextprotocol/tasks";

/** Reserved `_meta` keys carrying the per-request protocol fields. */
export const META_PROTOCOL_VERSION = "io.modelcontextprotocol/protocolVersion";
export const META_CLIENT_INFO = "io.modelcontextprotocol/clientInfo";
export const META_CLIENT_CAPABILITIES = "io.modelcontextprotocol/clientCapabilities";
export const META_SERVER_INFO = "io.modelcontextprotocol/serverInfo";

/** JSON-RPC error codes the protocol defines, plus the base ones a client acts on. */
export const RPC_METHOD_NOT_FOUND = -32601;
export const RPC_INVALID_PARAMS = -32602;
export const RPC_HEADER_MISMATCH = -32020;
export const RPC_MISSING_REQUIRED_CLIENT_CAPABILITY = -32021;
export const RPC_UNSUPPORTED_PROTOCOL_VERSION = -32022;

/** Polling cadence for a task whose server named none. */
const DEFAULT_TASK_POLL_INTERVAL_MS = 2000;

/**
 * How many times an `input_required` result that carries only `requestState` is
 * retried before the call is given up as never completing.
 */
const MAX_INPUT_REQUIRED_RETRIES = 3;

/** Methods whose `Mcp-Name` header mirrors a parameter, per the Streamable HTTP binding. */
const NAME_HEADER_SOURCE: Record<string, string> = {
  "tools/call": "name",
  "resources/read": "uri",
  "prompts/get": "name",
  "tasks/get": "taskId",
  "tasks/update": "taskId",
  "tasks/cancel": "taskId",
};

export interface Implementation {
  name: string;
  version: string;
  title?: string;
}

export interface ClientCapabilities {
  extensions?: Record<string, Record<string, unknown>>;
  [key: string]: unknown;
}

export interface ServerCapabilities {
  tools?: { listChanged?: boolean };
  resources?: { subscribe?: boolean; listChanged?: boolean };
  prompts?: { listChanged?: boolean };
  extensions?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface DiscoverResult {
  supportedVersions: string[];
  capabilities: ServerCapabilities;
  serverInfo?: Implementation;
  instructions?: string;
  ttlMs?: number;
  cacheScope?: string;
}

export interface McpToolDefinition {
  name: string;
  description?: string;
  inputSchema: Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
  annotations?: Record<string, unknown>;
  title?: string;
  [key: string]: unknown;
}

export interface McpListToolsResult {
  tools: McpToolDefinition[];
  nextCursor?: string;
  ttlMs?: number;
  cacheScope?: string;
}

export interface McpContentBlock {
  type: string;
  [key: string]: unknown;
}

export interface McpCallToolResult {
  content: McpContentBlock[];
  structuredContent?: unknown;
  isError?: boolean;
  [key: string]: unknown;
}

export interface JsonRpcErrorShape {
  code: number;
  message: string;
  data?: unknown;
}

export type TaskStatus =
  | "working"
  | "input_required"
  | "completed"
  | "failed"
  | "cancelled";

/** The task handle a `tools/call` answers with in place of its result. */
export interface McpTask {
  taskId: string;
  status: TaskStatus;
  statusMessage?: string;
  createdAt: string;
  lastUpdatedAt: string;
  ttlMs: number | null;
  pollIntervalMs?: number;
}

/** A polled task: the handle plus the payload its status carries. */
export interface McpTaskState extends McpTask {
  result?: Record<string, unknown>;
  error?: JsonRpcErrorShape;
  inputRequests?: Record<string, unknown>;
}

export interface McpProgress {
  progressToken: string | number;
  progress: number;
  total?: number;
  message?: string;
}

export interface McpNotification {
  method: string;
  params?: Record<string, unknown>;
}

export interface RequestOptions {
  signal?: AbortSignal;
  /** Receives the request-scoped notifications the response stream carries. */
  onNotification?: (notification: McpNotification) => void;
}

export interface CallToolOptions {
  signal?: AbortSignal;
  /** Progress the server reports against this call, while it runs inline. */
  onProgress?: (progress: McpProgress) => void;
  /** The call answered with a task handle; polling starts. */
  onTask?: (task: McpTask) => void;
  /** Each poll of that task, terminal state included. */
  onTaskUpdate?: (task: McpTaskState) => void;
}

export interface McpHttpClientOptions {
  /** The MCP endpoint, e.g. `https://api.dravr.ai/mcp`. */
  url: string | URL;
  clientInfo: Implementation;
  /** The bearer to send, resolved per request so a renewed session is picked up. */
  bearer?: () => Promise<string | undefined> | string | undefined;
  fetchFn?: typeof fetch;
  log?: (message: string) => void;
  /** Polling cadence when a task handle names none. */
  taskPollIntervalMs?: number;
}

/** A JSON-RPC error the server returned for a request. */
export class McpRpcError extends Error {
  readonly code: number;
  readonly data: unknown;

  constructor(code: number, message: string, data?: unknown) {
    super(message);
    this.name = "McpRpcError";
    this.code = code;
    this.data = data;
  }
}

/** An HTTP-level rejection: authentication, authorization, or a non-JSON-RPC failure. */
export class McpHttpError extends Error {
  readonly status: number;
  readonly wwwAuthenticate?: string;
  readonly rpc?: JsonRpcErrorShape;

  constructor(
    status: number,
    message: string,
    details: { wwwAuthenticate?: string; rpc?: JsonRpcErrorShape } = {},
  ) {
    super(message);
    this.name = "McpHttpError";
    this.status = status;
    this.wwwAuthenticate = details.wwwAuthenticate;
    this.rpc = details.rpc;
  }
}

/** A reply that does not fit the protocol: no response on the stream, an unknown result shape. */
export class McpProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "McpProtocolError";
  }
}

/** The server cancelled a task this call was waiting on. */
export class McpTaskCancelledError extends Error {
  readonly task: McpTaskState;

  constructor(task: McpTaskState) {
    super(
      task.statusMessage
        ? `Task ${task.taskId} was cancelled: ${task.statusMessage}`
        : `Task ${task.taskId} was cancelled`,
    );
    this.name = "McpTaskCancelledError";
    this.task = task;
  }
}

type JsonRpcMessage = {
  jsonrpc?: string;
  id?: string | number | null;
  method?: string;
  params?: Record<string, unknown>;
  result?: Record<string, unknown>;
  error?: JsonRpcErrorShape;
};

/** Matches a value HTTP can carry as a plain field value with no encoding. */
const PLAIN_HEADER_VALUE = /^[\x21-\x7e](?:[\x20-\x7e\t]*[\x21-\x7e])?$/;
const BASE64_SENTINEL = /^=\?base64\?.*\?=$/;

/**
 * Encodes a value for the `Mcp-Name` header per the binding's value encoding: plain
 * ASCII travels as is; anything else, and anything that already looks like the
 * sentinel, goes as `=?base64?...?=` over its UTF-8 bytes.
 */
export function encodeHeaderValue(value: string): string {
  if (PLAIN_HEADER_VALUE.test(value) && !BASE64_SENTINEL.test(value)) {
    return value;
  }
  return `=?base64?${Buffer.from(value, "utf8").toString("base64")}?=`;
}

function abortError(signal: AbortSignal): Error {
  const reason = signal.reason;
  if (reason instanceof Error) {
    return reason;
  }
  const error = new Error(
    typeof reason === "string" ? reason : "The operation was aborted",
  );
  error.name = "AbortError";
  return error;
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(abortError(signal));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    function onAbort() {
      clearTimeout(timer);
      reject(abortError(signal!));
    }
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function withoutResultType(result: Record<string, unknown>): Record<string, unknown> {
  const { resultType: _resultType, ...rest } = result;
  return rest;
}

/**
 * One MCP server reached over Streamable HTTP, revision 2026-07-28.
 *
 * There is no connection to open: each method here is a single POST that carries
 * everything the server needs. `discover()` is worth calling first all the same - it is
 * how the client learns whether the server serves the Tasks extension, which decides
 * whether `tools/call` may declare it and so be answered with a task handle.
 */
export class McpHttpClient {
  private readonly url: URL;
  private readonly clientInfo: Implementation;
  private readonly bearer?: McpHttpClientOptions["bearer"];
  private readonly fetchFn: typeof fetch;
  private readonly log: (message: string) => void;
  private readonly fallbackPollIntervalMs: number;
  private readonly inFlight = new Set<AbortController>();
  private nextId = 1;
  private discovered: DiscoverResult | undefined;
  private closed = false;

  constructor(options: McpHttpClientOptions) {
    this.url = new URL(String(options.url));
    this.clientInfo = options.clientInfo;
    this.bearer = options.bearer;
    this.fetchFn = options.fetchFn ?? fetch;
    this.log = options.log ?? (() => undefined);
    this.fallbackPollIntervalMs =
      options.taskPollIntervalMs ?? DEFAULT_TASK_POLL_INTERVAL_MS;
  }

  /** What the last `discover()` learned, if it ran. */
  get discovery(): DiscoverResult | undefined {
    return this.discovered;
  }

  get serverInfo(): Implementation | undefined {
    return this.discovered?.serverInfo;
  }

  get instructions(): string | undefined {
    return this.discovered?.instructions;
  }

  /** Whether the server advertised the Tasks extension on `server/discover`. */
  serverSupportsTasks(): boolean {
    const extensions = this.discovered?.capabilities?.extensions;
    return extensions !== undefined && extensions !== null && TASKS_EXTENSION_ID in extensions;
  }

  /**
   * The capabilities declared on each request. The Tasks extension is declared only
   * once the server has advertised it: a server never hands a task to a client that
   * did not declare the extension, and a client must not declare one the server does
   * not serve.
   */
  clientCapabilities(): ClientCapabilities {
    return this.serverSupportsTasks()
      ? { extensions: { [TASKS_EXTENSION_ID]: {} } }
      : {};
  }

  /** `server/discover`: supported revisions, capabilities and identity, kept for later calls. */
  async discover(signal?: AbortSignal): Promise<DiscoverResult> {
    const result = await this.request("server/discover", undefined, { signal });
    const discovered: DiscoverResult = {
      supportedVersions: Array.isArray(result.supportedVersions)
        ? (result.supportedVersions as string[])
        : [],
      capabilities: (result.capabilities as ServerCapabilities) ?? {},
      serverInfo:
        (result.serverInfo as Implementation | undefined) ??
        ((result._meta as Record<string, unknown> | undefined)?.[META_SERVER_INFO] as
          | Implementation
          | undefined),
      instructions: result.instructions as string | undefined,
      ttlMs: result.ttlMs as number | undefined,
      cacheScope: result.cacheScope as string | undefined,
    };
    this.discovered = discovered;
    return discovered;
  }

  /** `tools/list`, following `nextCursor` until the whole list is in hand. */
  async listTools(signal?: AbortSignal): Promise<McpListToolsResult> {
    const tools: McpToolDefinition[] = [];
    let firstPage: Record<string, unknown> | undefined;
    let cursor: string | undefined;
    do {
      const page = await this.request(
        "tools/list",
        cursor === undefined ? undefined : { cursor },
        { signal },
      );
      firstPage ??= page;
      tools.push(...((page.tools as McpToolDefinition[]) ?? []));
      cursor = typeof page.nextCursor === "string" ? page.nextCursor : undefined;
    } while (cursor !== undefined);

    const { nextCursor: _nextCursor, ...hints } = firstPage ?? {};
    return { ...hints, tools } as McpListToolsResult;
  }

  /**
   * `tools/call`, answered synchronously or through a task handle - the server decides
   * per call, and either way the caller gets the tool's result.
   */
  async callTool(
    params: { name: string; arguments?: Record<string, unknown> },
    options: CallToolOptions = {},
  ): Promise<McpCallToolResult> {
    const callParams: Record<string, unknown> = {
      name: params.name,
      arguments: params.arguments ?? {},
    };
    const progressToken = options.onProgress ? `call-${this.nextId}` : undefined;
    if (progressToken !== undefined) {
      callParams._meta = { progressToken };
    }
    const requestOptions: RequestOptions = {
      signal: options.signal,
      onNotification: (notification) => {
        if (notification.method === "notifications/progress" && options.onProgress) {
          const progress = notification.params as unknown as McpProgress;
          if (progress.progressToken === progressToken) {
            options.onProgress(progress);
          }
        }
      },
    };

    let retries = 0;
    let raw = await this.send("tools/call", callParams, requestOptions);
    for (;;) {
      const resultType = raw.resultType;
      if (resultType === undefined || resultType === "complete") {
        return withoutResultType(raw) as McpCallToolResult;
      }
      if (resultType === "task") {
        const task = withoutResultType(raw) as unknown as McpTask;
        options.onTask?.(task);
        return this.awaitTask(task, options);
      }
      if (resultType === "input_required") {
        const inputRequests = raw.inputRequests as Record<string, unknown> | undefined;
        const requested = inputRequests ? Object.keys(inputRequests) : [];
        if (requested.length > 0) {
          throw new McpProtocolError(
            `Tool ${params.name} asked for client input this client does not provide (${requested.join(", ")})`,
          );
        }
        if (retries >= MAX_INPUT_REQUIRED_RETRIES) {
          throw new McpProtocolError(
            `Tool ${params.name} kept answering input_required after ${retries} retries`,
          );
        }
        retries += 1;
        const retryParams = { ...callParams };
        if (typeof raw.requestState === "string") {
          retryParams.requestState = raw.requestState;
        }
        raw = await this.send("tools/call", retryParams, requestOptions);
        continue;
      }
      throw new McpProtocolError(
        `Tool ${params.name} answered with an unknown resultType ${JSON.stringify(resultType)}`,
      );
    }
  }

  async getTask(taskId: string, signal?: AbortSignal): Promise<McpTaskState> {
    const result = await this.request("tasks/get", { taskId }, { signal });
    return result as unknown as McpTaskState;
  }

  async updateTask(
    taskId: string,
    inputResponses: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<void> {
    await this.request("tasks/update", { taskId, inputResponses }, { signal });
  }

  async cancelTask(taskId: string, signal?: AbortSignal): Promise<void> {
    await this.request("tasks/cancel", { taskId }, { signal });
  }

  /**
   * Any request whose result is complete on its own. The `resultType` discriminator is
   * stripped from what comes back; a result of another shape is a protocol error here,
   * because only `callTool` knows how to carry a task or an input round-trip forward.
   */
  async request(
    method: string,
    params?: Record<string, unknown>,
    options: RequestOptions = {},
  ): Promise<Record<string, unknown>> {
    const raw = await this.send(method, params, options);
    const resultType = raw.resultType;
    if (resultType !== undefined && resultType !== "complete") {
      throw new McpProtocolError(
        `${method} answered with resultType ${JSON.stringify(resultType)}, which this request cannot carry forward`,
      );
    }
    return withoutResultType(raw);
  }

  /** Abandons every in-flight request. Closing the stream is the cancellation signal on HTTP. */
  close(): void {
    this.closed = true;
    for (const controller of this.inFlight) {
      controller.abort(new Error("Client closed"));
    }
    this.inFlight.clear();
  }

  private async awaitTask(
    task: McpTask,
    options: CallToolOptions,
  ): Promise<McpCallToolResult> {
    const signal = options.signal;
    let intervalMs = task.pollIntervalMs ?? this.fallbackPollIntervalMs;
    for (;;) {
      try {
        await sleep(intervalMs, signal);
      } catch (error) {
        await this.cancelQuietly(task.taskId);
        throw error;
      }

      let state: McpTaskState;
      try {
        state = await this.getTask(task.taskId, signal);
      } catch (error) {
        if (signal?.aborted) {
          await this.cancelQuietly(task.taskId);
        }
        throw error;
      }
      options.onTaskUpdate?.(state);
      intervalMs = state.pollIntervalMs ?? intervalMs;

      switch (state.status) {
        case "working":
          continue;
        case "completed":
          if (!state.result || typeof state.result !== "object") {
            throw new McpProtocolError(
              `Task ${task.taskId} completed without a result`,
            );
          }
          return withoutResultType(state.result) as McpCallToolResult;
        case "failed": {
          const error = state.error ?? {
            code: -32603,
            message: `Task ${task.taskId} failed without an error`,
          };
          throw new McpRpcError(error.code, error.message, error.data);
        }
        case "cancelled":
          throw new McpTaskCancelledError(state);
        case "input_required": {
          // This client declares no capability the server could ask through, so an
          // input request is unanswerable; cancel rather than leave the task waiting.
          await this.cancelQuietly(task.taskId);
          const requested = Object.keys(state.inputRequests ?? {});
          throw new McpProtocolError(
            `Task ${task.taskId} asked for client input this client does not provide (${requested.join(", ")})`,
          );
        }
        default:
          throw new McpProtocolError(
            `Task ${task.taskId} reported an unknown status ${JSON.stringify(state.status)}`,
          );
      }
    }
  }

  /** Cancels a task the caller stopped waiting on; the outcome is logged, never raised. */
  private async cancelQuietly(taskId: string): Promise<void> {
    try {
      await this.cancelTask(taskId, AbortSignal.timeout(5000));
      this.log(`Cancelled task ${taskId}`);
    } catch (error) {
      this.log(
        `Could not cancel task ${taskId}: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  /** One JSON-RPC request as one POST; returns the raw result, `resultType` and all. */
  private async send(
    method: string,
    params: Record<string, unknown> | undefined,
    options: RequestOptions,
  ): Promise<Record<string, unknown>> {
    if (this.closed) {
      throw new McpProtocolError("Client is closed");
    }

    const id = this.nextId++;
    const meta: Record<string, unknown> = {
      ...((params?._meta as Record<string, unknown> | undefined) ?? {}),
      [META_PROTOCOL_VERSION]: MCP_PROTOCOL_VERSION,
      [META_CLIENT_INFO]: this.clientInfo,
      [META_CLIENT_CAPABILITIES]: this.clientCapabilities(),
    };
    const body = {
      jsonrpc: "2.0",
      id,
      method,
      params: { ...(params ?? {}), _meta: meta },
    };

    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      Accept: "application/json, text/event-stream",
      "MCP-Protocol-Version": MCP_PROTOCOL_VERSION,
      "Mcp-Method": method,
    };
    const nameSource = NAME_HEADER_SOURCE[method];
    const nameValue = nameSource === undefined ? undefined : params?.[nameSource];
    if (typeof nameValue === "string") {
      headers["Mcp-Name"] = encodeHeaderValue(nameValue);
    }
    const bearer = await this.bearer?.();
    if (bearer) {
      headers.Authorization = `Bearer ${bearer}`;
    }

    const controller = new AbortController();
    const signal = options.signal
      ? AbortSignal.any([options.signal, controller.signal])
      : controller.signal;
    this.inFlight.add(controller);
    try {
      const response = await this.fetchFn(this.url, {
        method: "POST",
        headers,
        body: JSON.stringify(body),
        signal,
      });
      await this.rejectFailedStatus(response);
      const message = await this.readResponse(response, id, options.onNotification);
      if (message.error) {
        throw new McpRpcError(message.error.code, message.error.message, message.error.data);
      }
      if (!message.result || typeof message.result !== "object") {
        throw new McpProtocolError(`${method} answered without a result object`);
      }
      return message.result;
    } finally {
      this.inFlight.delete(controller);
    }
  }

  /** Turns a non-2xx status into the error that names what the server said. */
  private async rejectFailedStatus(response: Response): Promise<void> {
    if (response.ok) {
      return;
    }
    const rpc = await this.errorBody(response);
    if (response.status === 401 || response.status === 403) {
      throw new McpHttpError(
        response.status,
        rpc?.message ?? (response.status === 401 ? "Unauthorized" : "Forbidden"),
        {
          wwwAuthenticate: response.headers.get("www-authenticate") ?? undefined,
          rpc,
        },
      );
    }
    if (rpc) {
      // 400 for an unsupported version, a missing capability or a header mismatch;
      // 404 for an unknown method - each with the JSON-RPC error that says which.
      throw new McpRpcError(rpc.code, rpc.message, rpc.data);
    }
    throw new McpHttpError(response.status, `HTTP ${response.status} from ${this.url.origin}`);
  }

  private async errorBody(response: Response): Promise<JsonRpcErrorShape | undefined> {
    let text: string;
    try {
      text = await response.text();
    } catch {
      return undefined;
    }
    if (!text) {
      return undefined;
    }
    try {
      const parsed = JSON.parse(text) as JsonRpcMessage;
      if (parsed && typeof parsed === "object" && parsed.error && typeof parsed.error.code === "number") {
        return parsed.error;
      }
    } catch {
      // A body that is not JSON-RPC is described by its status alone.
    }
    return undefined;
  }

  /**
   * Reads the reply to request `id`: a single JSON object, or an SSE stream carrying
   * request-scoped notifications and then the response. The stream is released as
   * soon as the response is in hand.
   */
  private async readResponse(
    response: Response,
    id: number,
    onNotification: RequestOptions["onNotification"],
  ): Promise<JsonRpcMessage> {
    const contentType = response.headers.get("content-type") ?? "";
    let found: JsonRpcMessage | undefined;
    const consider = (message: JsonRpcMessage) => {
      if (message.id === id && (message.result !== undefined || message.error !== undefined)) {
        found = message;
        return;
      }
      if (message.method !== undefined && message.id === undefined) {
        onNotification?.({ method: message.method, params: message.params });
        return;
      }
      this.log(`Ignoring message on the response stream: ${JSON.stringify(message).slice(0, 200)}`);
    };

    if (contentType.includes("text/event-stream") && response.body) {
      await this.consumeEventStream(response.body, (message) => {
        consider(message);
        return found !== undefined;
      });
    } else {
      const text = await response.text();
      const parsed = JSON.parse(text) as JsonRpcMessage | JsonRpcMessage[];
      for (const message of Array.isArray(parsed) ? parsed : [parsed]) {
        consider(message);
      }
    }

    if (!found) {
      throw new McpProtocolError(`The reply to request ${id} carried no response`);
    }
    return found;
  }

  /** Feeds each SSE event's data to `deliver` until it reports the response arrived. */
  private async consumeEventStream(
    body: ReadableStream<Uint8Array>,
    deliver: (message: JsonRpcMessage) => boolean,
  ): Promise<void> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    const handleEvent = (raw: string): boolean => {
      const data = raw
        .split(/\r?\n/)
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).replace(/^ /, ""))
        .join("\n");
      if (!data) {
        return false;
      }
      const parsed = JSON.parse(data) as JsonRpcMessage | JsonRpcMessage[];
      for (const message of Array.isArray(parsed) ? parsed : [parsed]) {
        if (deliver(message)) {
          return true;
        }
      }
      return false;
    };

    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (value) {
          buffer += decoder.decode(value, { stream: true });
        }
        let boundary = buffer.search(/\r?\n\r?\n/);
        while (boundary !== -1) {
          const separator = buffer.slice(boundary).match(/^\r?\n\r?\n/)![0];
          const event = buffer.slice(0, boundary);
          buffer = buffer.slice(boundary + separator.length);
          if (handleEvent(event)) {
            return;
          }
          boundary = buffer.search(/\r?\n\r?\n/);
        }
        if (done) {
          buffer += decoder.decode();
          if (buffer.trim()) {
            handleEvent(buffer);
          }
          return;
        }
      }
    } finally {
      reader.cancel().catch(() => undefined);
    }
  }
}
