import * as React from "react";

// Per-chat automation preferences for the work panel, plus watermarks that
// keep the auto-prompts from repeating (one CI nudge per failing head sha,
// one comment nudge per count increase). Local state: the prompts are sent
// from this client into the chat, so they never need to sync.
const STORAGE_PREFIX = "buzz:chat-work-automation:v1";
const STORAGE_EVENT = "buzz:chat-work-automation-changed";

export type ChatWorkAutomation = {
  autoFixCi: boolean;
  addressComments: boolean;
  /** Head sha of the last CI failure the agent was asked to fix. */
  lastCiNudgeSha: string | null;
  /** Comment total at the last address-comments nudge. */
  lastCommentNudgeCount: number | null;
};

const DEFAULTS: ChatWorkAutomation = {
  autoFixCi: false,
  addressComments: false,
  lastCiNudgeSha: null,
  lastCommentNudgeCount: null,
};

function storageKey(chatId: string) {
  return `${STORAGE_PREFIX}:${chatId}`;
}

export function readChatWorkAutomation(chatId: string): ChatWorkAutomation {
  if (typeof window === "undefined") {
    return DEFAULTS;
  }
  try {
    const raw = window.localStorage.getItem(storageKey(chatId));
    if (!raw) {
      return DEFAULTS;
    }
    const parsed = JSON.parse(raw) as Partial<ChatWorkAutomation>;
    return {
      autoFixCi: Boolean(parsed.autoFixCi),
      addressComments: Boolean(parsed.addressComments),
      lastCiNudgeSha:
        typeof parsed.lastCiNudgeSha === "string"
          ? parsed.lastCiNudgeSha
          : null,
      lastCommentNudgeCount:
        typeof parsed.lastCommentNudgeCount === "number"
          ? parsed.lastCommentNudgeCount
          : null,
    };
  } catch {
    return DEFAULTS;
  }
}

export function updateChatWorkAutomation(
  chatId: string,
  patch: Partial<ChatWorkAutomation>,
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    const next = { ...readChatWorkAutomation(chatId), ...patch };
    window.localStorage.setItem(storageKey(chatId), JSON.stringify(next));
    window.dispatchEvent(new CustomEvent(STORAGE_EVENT));
  } catch {
    // Preferences are a convenience layer; ignore unavailable storage.
  }
}

export function useChatWorkAutomation(chatId: string): ChatWorkAutomation {
  const [state, setState] = React.useState(() =>
    readChatWorkAutomation(chatId),
  );

  React.useEffect(() => {
    const refresh = () => setState(readChatWorkAutomation(chatId));
    refresh();
    window.addEventListener(STORAGE_EVENT, refresh);
    window.addEventListener("storage", refresh);
    return () => {
      window.removeEventListener(STORAGE_EVENT, refresh);
      window.removeEventListener("storage", refresh);
    };
  }, [chatId]);

  return state;
}
