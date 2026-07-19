#!/usr/bin/env node
/** Deterministic ACP fixture for agents-everywhere live tests. */
import { createInterface } from "node:readline";

const wakeDelayMs = Number.parseInt(
  process.env.BUZZ_E2E_FAKE_ACP_WAKE_MS ?? "0",
  10,
);
if (!Number.isFinite(wakeDelayMs) || wakeDelayMs < 0) {
  throw new Error("BUZZ_E2E_FAKE_ACP_WAKE_MS must be a non-negative integer");
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const write = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
const textFromPrompt = (params) =>
  (params?.prompt ?? [])
    .filter((part) => part?.type === "text" && typeof part.text === "string")
    .map((part) => part.text)
    .join("\n");

let sessionCounter = 0;
const input = createInterface({
  input: process.stdin,
  crlfDelay: Number.POSITIVE_INFINITY,
});
for await (const line of input) {
  if (!line.trim()) continue;
  const request = JSON.parse(line);
  if (request.id === undefined || typeof request.method !== "string") continue;

  switch (request.method) {
    case "initialize":
      if (wakeDelayMs > 0) await sleep(wakeDelayMs);
      write({
        jsonrpc: "2.0",
        id: request.id,
        result: { protocolVersion: 2, agentCapabilities: {} },
      });
      break;
    case "session/new":
      sessionCounter += 1;
      write({
        jsonrpc: "2.0",
        id: request.id,
        result: { sessionId: `fake-session-${sessionCounter}` },
      });
      break;
    case "session/prompt": {
      const prompt = textFromPrompt(request.params);
      const ids = [...prompt.matchAll(/\bAE-ID:([A-Za-z0-9._:-]+)\b/g)].map(
        (match) => match[1],
      );
      write({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: request.params?.sessionId,
          update: {
            sessionUpdate: "agent_message_chunk",
            content: { type: "text", text: `AE-ACK:${ids.join(",")}` },
          },
        },
      });
      write({
        jsonrpc: "2.0",
        id: request.id,
        result: { stopReason: "end_turn" },
      });
      break;
    }
    case "session/cancel":
      write({ jsonrpc: "2.0", id: request.id, result: {} });
      break;
    default:
      write({
        jsonrpc: "2.0",
        id: request.id,
        error: {
          code: -32601,
          message: `Unsupported fixture method: ${request.method}`,
        },
      });
  }
}
