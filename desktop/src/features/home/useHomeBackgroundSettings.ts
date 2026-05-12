import * as React from "react";

export type HomeBackgroundMode = "none" | "image" | "video" | "animation";

export type HomeBackgroundSettings = {
  mode: HomeBackgroundMode;
  sourceUrl: string;
  animationCode: string;
  opacity: number;
  scrimOpacity: number;
};

const STORAGE_KEY = "sprout:home-background:v1";
const SETTINGS_EVENT = "sprout:home-background-settings-changed";
let cachedRawValue: string | null | undefined;
let cachedSettings: HomeBackgroundSettings | undefined;

export const DEFAULT_HOME_BACKGROUND_SETTINGS: HomeBackgroundSettings = {
  mode: "none",
  sourceUrl: "",
  animationCode: "",
  opacity: 0.45,
  scrimOpacity: 0.55,
};

function clampUnit(value: unknown, fallback: number) {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.min(1, Math.max(0, value))
    : fallback;
}

function isHomeBackgroundMode(value: unknown): value is HomeBackgroundMode {
  return (
    value === "none" ||
    value === "image" ||
    value === "video" ||
    value === "animation"
  );
}

function parseHomeBackgroundSettings(
  value: unknown,
): HomeBackgroundSettings | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }

  const record = value as Record<string, unknown>;
  const mode = isHomeBackgroundMode(record.mode) ? record.mode : "none";

  return {
    mode,
    sourceUrl:
      typeof record.sourceUrl === "string" ? record.sourceUrl.trim() : "",
    animationCode:
      typeof record.animationCode === "string" ? record.animationCode : "",
    opacity: clampUnit(
      record.opacity,
      DEFAULT_HOME_BACKGROUND_SETTINGS.opacity,
    ),
    scrimOpacity: clampUnit(
      record.scrimOpacity,
      DEFAULT_HOME_BACKGROUND_SETTINGS.scrimOpacity,
    ),
  };
}

export function readHomeBackgroundSettings(): HomeBackgroundSettings {
  if (typeof window === "undefined") {
    return DEFAULT_HOME_BACKGROUND_SETTINGS;
  }

  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === cachedRawValue && cachedSettings) {
      return cachedSettings;
    }

    cachedRawValue = raw;

    if (!raw) {
      cachedSettings = DEFAULT_HOME_BACKGROUND_SETTINGS;
      return cachedSettings;
    }

    cachedSettings =
      parseHomeBackgroundSettings(JSON.parse(raw)) ??
      DEFAULT_HOME_BACKGROUND_SETTINGS;
    return cachedSettings;
  } catch {
    cachedSettings = DEFAULT_HOME_BACKGROUND_SETTINGS;
    return cachedSettings;
  }
}

function writeHomeBackgroundSettings(settings: HomeBackgroundSettings) {
  const raw = JSON.stringify(settings);
  cachedRawValue = raw;
  cachedSettings = settings;
  window.localStorage.setItem(STORAGE_KEY, raw);
  window.dispatchEvent(new Event(SETTINGS_EVENT));
}

function subscribeHomeBackgroundSettings(onStoreChange: () => void) {
  window.addEventListener(SETTINGS_EVENT, onStoreChange);
  window.addEventListener("storage", onStoreChange);

  return () => {
    window.removeEventListener(SETTINGS_EVENT, onStoreChange);
    window.removeEventListener("storage", onStoreChange);
  };
}

export function useHomeBackgroundSettings() {
  const settings = React.useSyncExternalStore(
    subscribeHomeBackgroundSettings,
    readHomeBackgroundSettings,
    () => DEFAULT_HOME_BACKGROUND_SETTINGS,
  );

  const setSettings = React.useCallback(
    (
      next:
        | HomeBackgroundSettings
        | ((current: HomeBackgroundSettings) => HomeBackgroundSettings),
    ) => {
      const current = readHomeBackgroundSettings();
      const resolved = typeof next === "function" ? next(current) : next;
      writeHomeBackgroundSettings(
        parseHomeBackgroundSettings(resolved) ??
          DEFAULT_HOME_BACKGROUND_SETTINGS,
      );
    },
    [],
  );

  const resetSettings = React.useCallback(() => {
    writeHomeBackgroundSettings(DEFAULT_HOME_BACKGROUND_SETTINGS);
  }, []);

  return { settings, setSettings, resetSettings };
}

export default useHomeBackgroundSettings;
