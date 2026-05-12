import { Image } from "lucide-react";
import { useState } from "react";

import {
  type HomeBackgroundMode,
  useHomeBackgroundSettings,
} from "@/features/home/useHomeBackgroundSettings";
import { HomeBackgroundLayer } from "@/features/home/ui/HomeBackgroundLayer";
import { pickHomeBackgroundFile } from "@/shared/api/tauri";
import { Textarea } from "@/shared/ui/textarea";

const backgroundModeOptions: { value: HomeBackgroundMode; label: string }[] = [
  { value: "none", label: "None" },
  { value: "image", label: "Image URL" },
  { value: "video", label: "Video URL" },
  { value: "animation", label: "Sandboxed animation" },
];

export function HomeBackgroundSettingsCard() {
  const {
    settings: homeBackgroundSettings,
    setSettings: setHomeBackgroundSettings,
    resetSettings: resetHomeBackgroundSettings,
  } = useHomeBackgroundSettings();
  const [backgroundUploadError, setBackgroundUploadError] = useState<
    string | null
  >(null);
  const [pickingBackgroundKind, setPickingBackgroundKind] = useState<
    "image" | "video" | "script" | null
  >(null);

  async function handlePickHomeBackgroundFile(
    kind: "image" | "video" | "script",
  ) {
    setBackgroundUploadError(null);
    setPickingBackgroundKind(kind);

    try {
      const file = await pickHomeBackgroundFile(kind);
      if (!file) return;

      if (file.kind === "script") {
        setHomeBackgroundSettings((current) => ({
          ...current,
          animationCode: file.content,
          mode: "animation",
        }));
        return;
      }

      const mode = kind === "video" ? "video" : "image";
      setHomeBackgroundSettings((current) => ({
        ...current,
        mode,
        sourceUrl: file.sourceUrl,
      }));
    } catch (error) {
      setBackgroundUploadError(
        error instanceof Error ? error.message : "Failed to choose file.",
      );
    } finally {
      setPickingBackgroundKind(null);
    }
  }

  return (
    <div className="rounded-xl border border-border/70 bg-background/70 p-4">
      <div className="mb-4 flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
          <Image className="h-4 w-4" />
        </div>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold tracking-tight">
            Home background
          </h3>
          <p className="mt-1 text-sm text-muted-foreground">
            Add a full-screen visual background to Search, Inbox, and Feed.
            Custom animations run in a sandboxed, visual-only iframe.
          </p>
        </div>
      </div>

      <div className="grid gap-4">
        <label className="grid gap-1.5 text-sm">
          <span className="font-medium">Background type</span>
          <select
            className="h-9 rounded-md border border-input bg-background px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            onChange={(event) => {
              const mode = event.target.value as HomeBackgroundMode;
              setHomeBackgroundSettings((current) => ({ ...current, mode }));
            }}
            value={homeBackgroundSettings.mode}
          >
            {backgroundModeOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        {homeBackgroundSettings.mode === "image" ||
        homeBackgroundSettings.mode === "video" ? (
          <div className="grid gap-1.5 text-sm">
            <div className="flex items-center justify-between gap-3">
              <label className="font-medium" htmlFor="home-background-url">
                {homeBackgroundSettings.mode === "image"
                  ? "Image URL"
                  : "Video URL"}
              </label>
              <button
                className="inline-flex h-8 items-center rounded-md border border-border/70 px-3 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-60"
                disabled={pickingBackgroundKind !== null}
                onClick={() => {
                  void handlePickHomeBackgroundFile(
                    homeBackgroundSettings.mode === "image" ? "image" : "video",
                  );
                }}
                type="button"
              >
                {pickingBackgroundKind === homeBackgroundSettings.mode
                  ? "Choosing..."
                  : `Choose ${homeBackgroundSettings.mode}`}
              </button>
            </div>
            <input
              id="home-background-url"
              className="h-9 rounded-md border border-input bg-background px-3 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              onChange={(event) => {
                setHomeBackgroundSettings((current) => ({
                  ...current,
                  sourceUrl: event.target.value,
                }));
              }}
              placeholder={
                homeBackgroundSettings.mode === "image"
                  ? "https://example.com/background.jpg"
                  : "https://example.com/background.mp4"
              }
              type="url"
              value={homeBackgroundSettings.sourceUrl}
            />
          </div>
        ) : null}

        {homeBackgroundSettings.mode === "animation" ? (
          <div className="grid gap-1.5 text-sm">
            <div className="flex items-center justify-between gap-3">
              <label
                className="font-medium"
                htmlFor="home-background-animation"
              >
                HTML, CSS, and JavaScript
              </label>
              <button
                className="inline-flex h-8 items-center rounded-md border border-border/70 px-3 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-60"
                disabled={pickingBackgroundKind !== null}
                onClick={() => {
                  void handlePickHomeBackgroundFile("script");
                }}
                type="button"
              >
                {pickingBackgroundKind === "script"
                  ? "Choosing..."
                  : "Choose script"}
              </button>
            </div>
            <Textarea
              id="home-background-animation"
              className="min-h-40 font-mono text-xs"
              onChange={(event) => {
                setHomeBackgroundSettings((current) => ({
                  ...current,
                  animationCode: event.target.value,
                }));
              }}
              placeholder="<canvas id='bg'></canvas><script>/* visual-only animation */</script>"
              spellCheck={false}
              value={homeBackgroundSettings.animationCode}
            />
            <span className="text-xs text-muted-foreground">
              Scripts are sandboxed away from Sprout and external network
              requests are blocked by the iframe policy.
            </span>
          </div>
        ) : null}

        <div className="grid gap-3 sm:grid-cols-2">
          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">
              Background opacity{" "}
              {Math.round(homeBackgroundSettings.opacity * 100)}%
            </span>
            <input
              max="1"
              min="0"
              onChange={(event) => {
                setHomeBackgroundSettings((current) => ({
                  ...current,
                  opacity: Number(event.target.value),
                }));
              }}
              step="0.05"
              type="range"
              value={homeBackgroundSettings.opacity}
            />
          </label>

          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">
              Readability overlay{" "}
              {Math.round(homeBackgroundSettings.scrimOpacity * 100)}%
            </span>
            <input
              max="1"
              min="0"
              onChange={(event) => {
                setHomeBackgroundSettings((current) => ({
                  ...current,
                  scrimOpacity: Number(event.target.value),
                }));
              }}
              step="0.05"
              type="range"
              value={homeBackgroundSettings.scrimOpacity}
            />
          </label>
        </div>

        <div className="relative h-32 overflow-hidden rounded-xl border border-border/70 bg-muted/30">
          <HomeBackgroundLayer settings={homeBackgroundSettings} />
          <div className="relative z-10 flex h-full items-center justify-center p-4">
            <div className="rounded-2xl border border-border/70 bg-background/75 px-4 py-3 text-center shadow-sm backdrop-blur">
              <p className="text-sm font-medium">Home preview</p>
              <p className="mt-1 text-xs text-muted-foreground">
                This background appears behind all Home tabs.
              </p>
            </div>
          </div>
        </div>

        <div className="flex justify-end">
          <button
            className="inline-flex h-8 items-center rounded-md border border-border/70 px-3 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
            onClick={resetHomeBackgroundSettings}
            type="button"
          >
            Reset background
          </button>
        </div>

        {backgroundUploadError ? (
          <p className="text-xs text-destructive">{backgroundUploadError}</p>
        ) : null}

        {homeBackgroundSettings.mode !== "none" &&
        homeBackgroundSettings.mode !== "animation" &&
        homeBackgroundSettings.sourceUrl.trim().length === 0 ? (
          <p className="text-xs text-destructive">
            Add a URL before this background can render.
          </p>
        ) : null}

        {homeBackgroundSettings.mode === "animation" &&
        homeBackgroundSettings.animationCode.trim().length === 0 ? (
          <p className="text-xs text-destructive">
            Add animation markup before this background can render.
          </p>
        ) : null}
      </div>
    </div>
  );
}
