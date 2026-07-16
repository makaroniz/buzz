import { expect, test } from "@playwright/test";

test("announcement demo loads its workspace, people, and projects", async ({
  page,
}) => {
  const agentReply =
    "I’d lead with the handoff moment, then land on the shared launch room. That gives the story a clear before-and-after.";
  await page.route("**/__announcement-demo/agent-response", async (route) => {
    await route.fulfill({ json: { text: agentReply } });
  });
  await page.goto("/?demo=announcement");
  await page.waitForFunction(
    () =>
      typeof (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  await page.evaluate(async () => {
    const invoke = (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: unknown,
        ) => Promise<unknown>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) {
      throw new Error("Announcement demo command bridge is unavailable.");
    }
    await invoke("set_global_agent_config", {
      config: {
        env_vars: { OPENAI_COMPAT_API_KEY: "e2e-demo-key" },
        provider: "openai",
        model: "gpt-5.4-mini",
      },
    });
  });

  await expect(page.getByText("The Hive", { exact: true })).toBeVisible();
  await expect(page.getByText("Product", { exact: true })).toBeVisible();
  await expect(page.getByText("Launch Swarm", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Honeycomb Studios", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Alex Rivera", { exact: true }).last(),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-DM").filter({ hasText: "Maya Chen" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-DM").filter({ hasText: "Jordan Brooks" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-DM").filter({ hasText: "Priya Shah" }),
  ).toBeVisible();
  await expect(page.getByTestId("channel-DM")).toHaveCount(3);

  await page.getByTestId("channel-flight-path").click();
  const channelTimeline = page.getByTestId("message-timeline");
  await expect(channelTimeline).toContainText("Marcus Reed");
  await expect(channelTimeline).toContainText("Elena Torres");
  await expect(channelTimeline).toContainText("Perfect. That’s the move.");
  const demoBuildRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Demo build is running" })
    .last();
  await expect(
    demoBuildRow.locator('[data-link-preview="github-pull-request"]'),
  ).toBeVisible();
  await expect(demoBuildRow.getByTestId("message-reactions")).toContainText(
    "✅",
  );
  await expect(
    channelTimeline.locator('[data-link-preview="linear-issue"]').last(),
  ).toBeVisible();

  const channelMessage = `The recording pass is ready ${Date.now()}`;
  await page.getByTestId("message-input").fill(channelMessage);
  await page.getByTestId("send-message").click();
  await expect(channelTimeline).toContainText(channelMessage);

  const messageInput = page.getByTestId("message-input");
  await messageInput.fill("Could ");
  await messageInput.pressSequentially("@Sco");
  const agentMention = page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete")
    .locator("button", { hasText: "Scout" });
  await expect(agentMention).toBeVisible();
  await agentMention.click();
  await messageInput.pressSequentially(" suggest the strongest story beat?");
  await page.getByTestId("send-message").click();
  await expect(channelTimeline).toContainText(agentReply, { timeout: 10_000 });

  const populatedChannels = [
    ["announcements", "Final smoke pass is clean"],
    ["general", "Please nobody breathe on main"],
    ["design", "Looks great on camera"],
    ["mobile", "The draft follows you now"],
    ["marketing", "No copy-paste script"],
    ["queen-bee-launch", "Sound mix is approved"],
  ] as const;
  for (const [channel, excerpt] of populatedChannels) {
    await page.getByTestId(`channel-${channel}`).click();
    await expect(channelTimeline).toContainText(excerpt);
  }

  await page.getByTestId("channel-design").click();
  await expect(channelTimeline.getByAltText("image").last()).toBeVisible();
  await expect(
    channelTimeline.locator('[data-link-preview="google-docs-document"]'),
  ).toBeVisible();

  await page.getByTestId("channel-marketing").click();
  await expect(
    channelTimeline
      .getByTestId("file-card")
      .filter({ hasText: "launch-social-crops.zip" }),
  ).toBeVisible();
  await expect(
    channelTimeline.locator('[data-link-preview="google-sheets-spreadsheet"]'),
  ).toBeVisible();

  await page.getByTestId("channel-DM").filter({ hasText: "Maya Chen" }).click();
  const dmMessage = `Can you join the capture review? ${Date.now()}`;
  await page.getByTestId("message-input").fill(dmMessage);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(dmMessage);

  await page.goto("/?demo=announcement#/projects");
  await page.locator('button[aria-label="Repositories"]').click();
  for (const project of ["flight-path", "nectar", "comb-kit", "swarm-launch"]) {
    await expect(
      page.locator(
        `[data-testid="project-card-${project}"], [data-testid="project-row-${project}"]`,
      ),
    ).toBeVisible();
  }
});
