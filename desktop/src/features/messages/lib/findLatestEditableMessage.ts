import type { TimelineMessage } from "@/features/messages/types";

export function findLatestEditableMessage(
  messages: TimelineMessage[],
  currentPubkey: string | undefined,
): TimelineMessage | null {
  if (!currentPubkey) {
    return null;
  }

  const normalizedCurrentPubkey = currentPubkey.toLowerCase();
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.pubkey?.toLowerCase() === normalizedCurrentPubkey) {
      return message;
    }
  }

  return null;
}
