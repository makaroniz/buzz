import type { Plugin } from "vite";

const AGENT_RESPONSE_PATH = "/__announcement-demo/agent-response";
const MAX_REQUEST_BYTES = 64 * 1024;
const REQUEST_TIMEOUT_MS = 45_000;

type AnnouncementDemoAgentMessage = {
  role: "assistant" | "user";
  content: string;
};

type AnnouncementDemoAgentRequest = {
  provider: "anthropic" | "openai";
  apiKey: string;
  model: string;
  systemPrompt: string;
  messages: AnnouncementDemoAgentMessage[];
};

type ProviderErrorBody = {
  error?: string | { message?: string };
  message?: string;
};

function isAgentMessage(value: unknown): value is AnnouncementDemoAgentMessage {
  if (typeof value !== "object" || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return (
    (candidate.role === "assistant" || candidate.role === "user") &&
    typeof candidate.content === "string" &&
    candidate.content.trim().length > 0
  );
}

function parseAgentRequest(value: unknown): AnnouncementDemoAgentRequest {
  if (typeof value !== "object" || value === null) {
    throw new Error("The agent request was not valid JSON.");
  }

  const candidate = value as Record<string, unknown>;
  if (candidate.provider !== "anthropic" && candidate.provider !== "openai") {
    throw new Error("Choose Anthropic or OpenAI as the agent provider.");
  }
  if (typeof candidate.apiKey !== "string" || !candidate.apiKey.trim()) {
    throw new Error("Add an API key in the agent settings first.");
  }
  if (typeof candidate.model !== "string" || !candidate.model.trim()) {
    throw new Error("Choose a model in the agent settings first.");
  }
  if (
    typeof candidate.systemPrompt !== "string" ||
    !Array.isArray(candidate.messages) ||
    !candidate.messages.every(isAgentMessage)
  ) {
    throw new Error("The agent conversation context was incomplete.");
  }

  return {
    provider: candidate.provider,
    apiKey: candidate.apiKey.trim(),
    model: candidate.model.trim(),
    systemPrompt: candidate.systemPrompt,
    messages: candidate.messages,
  };
}

function readRequestBody(request: NodeJS.ReadableStream): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let bytesRead = 0;

    request.on("data", (chunk: Buffer) => {
      bytesRead += chunk.length;
      if (bytesRead > MAX_REQUEST_BYTES) {
        reject(new Error("The agent conversation was too large to send."));
        return;
      }
      chunks.push(chunk);
    });
    request.on("end", () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
      } catch {
        reject(new Error("The agent request was not valid JSON."));
      }
    });
    request.on("error", reject);
  });
}

function normalizeAnthropicModel(model: string) {
  if (model === "goose-claude-4-6-sonnet") {
    return "claude-sonnet-4-6";
  }
  if (model === "goose-claude-4-6-opus") {
    return "claude-opus-4-6";
  }
  return model.replace(/^anthropic\//, "");
}

function extractProviderError(body: ProviderErrorBody, status: number) {
  const nestedMessage =
    typeof body.error === "object" ? body.error.message : undefined;
  const message = nestedMessage ?? body.message ?? body.error;
  if (typeof message === "string" && message.trim()) {
    return `Provider request failed (${status}): ${message.trim()}`;
  }
  return `Provider request failed with status ${status}.`;
}

function extractOpenAiText(body: unknown) {
  if (typeof body !== "object" || body === null) {
    return null;
  }

  const response = body as {
    output_text?: unknown;
    output?: Array<{
      content?: Array<{ type?: unknown; text?: unknown }>;
    }>;
  };
  if (typeof response.output_text === "string" && response.output_text.trim()) {
    return response.output_text.trim();
  }

  const text = (response.output ?? [])
    .flatMap((item) => item.content ?? [])
    .filter((part) => part.type === "output_text")
    .map((part) => (typeof part.text === "string" ? part.text : ""))
    .join("")
    .trim();
  return text || null;
}

function extractAnthropicText(body: unknown) {
  if (typeof body !== "object" || body === null) {
    return null;
  }

  const response = body as {
    content?: Array<{ type?: unknown; text?: unknown }>;
  };
  const text = (response.content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => (typeof part.text === "string" ? part.text : ""))
    .join("")
    .trim();
  return text || null;
}

async function requestOpenAiResponse(input: AnnouncementDemoAgentRequest) {
  const response = await fetch("https://api.openai.com/v1/responses", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${input.apiKey}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model: input.model,
      instructions: input.systemPrompt,
      input: input.messages,
      max_output_tokens: 350,
      store: false,
    }),
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  const body = (await response.json()) as unknown;
  if (!response.ok) {
    throw new Error(
      extractProviderError(body as ProviderErrorBody, response.status),
    );
  }

  const text = extractOpenAiText(body);
  if (!text) {
    throw new Error("OpenAI returned a response without any text.");
  }
  return text;
}

async function requestAnthropicResponse(input: AnnouncementDemoAgentRequest) {
  const response = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "anthropic-version": "2023-06-01",
      "Content-Type": "application/json",
      "x-api-key": input.apiKey,
    },
    body: JSON.stringify({
      model: normalizeAnthropicModel(input.model),
      max_tokens: 350,
      system: input.systemPrompt,
      messages: input.messages,
    }),
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  const body = (await response.json()) as unknown;
  if (!response.ok) {
    throw new Error(
      extractProviderError(body as ProviderErrorBody, response.status),
    );
  }

  const text = extractAnthropicText(body);
  if (!text) {
    throw new Error("Anthropic returned a response without any text.");
  }
  return text;
}

function friendlyError(error: unknown) {
  if (error instanceof Error && error.name === "TimeoutError") {
    return "The model took too long to respond. Please try again.";
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "The model could not respond just now.";
}

/**
 * Local-only provider proxy for the announcement demo. Keeping the API request
 * in Vite means provider credentials are never built into the frontend bundle
 * or written to source, and no Docker-backed relay is required.
 */
export function announcementDemoAgentPlugin(): Plugin {
  return {
    name: "buzz-announcement-demo-agent",
    configureServer(server) {
      server.middlewares.use(AGENT_RESPONSE_PATH, async (request, response) => {
        response.setHeader("Content-Type", "application/json");
        response.setHeader("Cache-Control", "no-store");

        if (request.method !== "POST") {
          response.statusCode = 405;
          response.end(JSON.stringify({ error: "Method not allowed." }));
          return;
        }

        try {
          const input = parseAgentRequest(await readRequestBody(request));
          const text =
            input.provider === "openai"
              ? await requestOpenAiResponse(input)
              : await requestAnthropicResponse(input);
          response.statusCode = 200;
          response.end(JSON.stringify({ text }));
        } catch (error) {
          response.statusCode = 502;
          response.end(JSON.stringify({ error: friendlyError(error) }));
        }
      });
    },
  };
}
