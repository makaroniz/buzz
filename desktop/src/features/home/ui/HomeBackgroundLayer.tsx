import * as React from "react";

import type { HomeBackgroundSettings } from "@/features/home/useHomeBackgroundSettings";
import { cn } from "@/shared/lib/cn";

type HomeBackgroundLayerProps = {
  className?: string;
  settings: HomeBackgroundSettings;
};

function buildAnimationSrcDoc(code: string) {
  return `<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:;" />
  <style>
    html, body {
      width: 100%;
      height: 100%;
      margin: 0;
      overflow: hidden;
      background: transparent;
    }

    *, *::before, *::after {
      box-sizing: border-box;
    }
  </style>
</head>
<body>
${code}
</body>
</html>`;
}

function hasRenderableBackground(settings: HomeBackgroundSettings) {
  if (settings.mode === "none") {
    return false;
  }

  if (settings.mode === "animation") {
    return settings.animationCode.trim().length > 0;
  }

  return settings.sourceUrl.trim().length > 0;
}

export function HomeBackgroundLayer({
  className,
  settings,
}: HomeBackgroundLayerProps) {
  const mediaSource =
    settings.mode === "image" || settings.mode === "video"
      ? settings.sourceUrl.trim()
      : "";
  const [failedMediaSource, setFailedMediaSource] = React.useState<
    string | null
  >(null);

  if (!hasRenderableBackground(settings)) {
    return null;
  }

  if (mediaSource && failedMediaSource === mediaSource) {
    return null;
  }

  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-0 overflow-hidden",
        className,
      )}
    >
      {settings.mode === "image" ? (
        <img
          alt=""
          className="h-full w-full object-cover"
          onError={() => setFailedMediaSource(mediaSource)}
          src={settings.sourceUrl}
          style={{ opacity: settings.opacity }}
        />
      ) : null}

      {settings.mode === "video" ? (
        <video
          autoPlay
          className="h-full w-full object-cover"
          loop
          muted
          onError={() => setFailedMediaSource(mediaSource)}
          playsInline
          src={settings.sourceUrl}
          style={{ opacity: settings.opacity }}
        />
      ) : null}

      {settings.mode === "animation" ? (
        <iframe
          className="h-full w-full border-0"
          sandbox="allow-scripts"
          srcDoc={buildAnimationSrcDoc(settings.animationCode)}
          style={{ opacity: settings.opacity }}
          title="Home background animation"
        />
      ) : null}

      <div
        className="absolute inset-0 bg-background"
        style={{ opacity: settings.scrimOpacity }}
      />
    </div>
  );
}
