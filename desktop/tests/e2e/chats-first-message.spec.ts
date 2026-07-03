import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("first message in a new chat is sent and rendered", async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  page.on("pageerror", (error) => {
    consoleErrors.push(`pageerror: ${error.message}`);
  });

  await installMockBridge(page);
  await page.goto("/#/chats");

  const composer = page.locator("[contenteditable='true'], textarea").first();
  await expect(composer).toBeVisible();
  await composer.click();
  await composer.fill("Hello Fizz, first message");
  await composer.press("Enter");

  // Give the create+send flow time to run.
  await page.waitForTimeout(4_000);

  console.log("URL after send:", page.url());
  console.log("Console errors:", JSON.stringify(consoleErrors, null, 2));

  await page.screenshot({
    path: "test-results/chats-first-message.png",
    fullPage: false,
  });

  // The chat should have been created and navigated to...
  await expect(page).toHaveURL(/\/chats\/.+/);
  // ...and the first message should be visible in the conversation (the same
  // text also appears in the sidebar item and chat title, so scope the
  // assertion to the message list).
  await expect(
    page.getByLabel("Chat messages").getByText("Hello Fizz, first message"),
  ).toBeVisible();

  // Just-sent rows slide in via the entrance animation (recency-gated, so
  // only fresh messages animate — not history on chat open).
  const animatedRow = page
    .getByLabel("Chat messages")
    .locator(".buzz-message-entrance", { hasText: "Hello Fizz, first message" })
    .first();
  await expect(animatedRow).toBeVisible();
  expect(
    await animatedRow.evaluate(
      (element) => window.getComputedStyle(element).animationName,
    ),
  ).toBe("buzz-message-entrance");

  // Once the conversation develops (two agent replies), the chat auto-titles
  // itself with a succinct subject line ("Hello Fizz, first message" →
  // "First message"), rendered through the animated title swap.
  await expect
    .poll(async () =>
      page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: "Hello Fizz, first message" },
      ),
    )
    .toBe(true);

  const fizzPubkey = await page.evaluate(async () => {
    const win = window as Window & {
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
        command: string,
        payload: unknown,
      ) => Promise<unknown>;
    };
    const agents = (await win.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "list_managed_agents",
      {},
    )) as Array<{ name: string; pubkey: string }> | undefined;
    return agents?.find((agent) => agent.name === "Fizz")?.pubkey ?? null;
  });
  expect(fizzPubkey).toBeTruthy();

  await page.evaluate(
    ({ pubkey }) => {
      const win = window as Window & {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          pubkey?: string;
        }) => unknown;
      };
      win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "Hello Fizz, first message",
        content: "Sure — that first message says hello.",
        pubkey: pubkey ?? undefined,
      });
      win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "Hello Fizz, first message",
        content: "Anything else you want to check while we're here?",
        pubkey: pubkey ?? undefined,
      });
    },
    { pubkey: fizzPubkey },
  );

  await expect(
    page.getByRole("heading", { name: "First message" }),
  ).toBeVisible({ timeout: 10_000 });
  const animatedTitle = page
    .getByTestId("chat-title")
    .getByTestId("animated-title-text");
  await expect(animatedTitle).toBeVisible();
  // The measured width must fit the whole title — a collapsed box renders
  // "F…" instead of the text.
  await expect
    .poll(async () =>
      animatedTitle.evaluate((element) => {
        const sizer = element.querySelector<HTMLElement>(
          "span[aria-hidden='true']",
        );
        if (!sizer) return -1;
        return element.getBoundingClientRect().width - sizer.scrollWidth;
      }),
    )
    .toBeGreaterThanOrEqual(-1);
});
