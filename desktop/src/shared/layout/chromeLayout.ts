export const TOP_CHROME_HEIGHT_DEFAULT = "2.5rem";
export const CHANNEL_CONTENT_TOP_PADDING_DEFAULT = "5.75rem";
export const INSET_HEADER_HEIGHT_DEFAULT = "5rem";

export const chromeCssVars = {
  topChromeHeight: "--buzz-top-chrome-height",
  channelContentTopPadding: "--buzz-channel-content-top-padding",
  insetHeaderHeight: "--buzz-inset-header-height",
} as const;

export const chromeCssVarDefaults = {
  [chromeCssVars.topChromeHeight]: TOP_CHROME_HEIGHT_DEFAULT,
  [chromeCssVars.channelContentTopPadding]: CHANNEL_CONTENT_TOP_PADDING_DEFAULT,
  [chromeCssVars.insetHeaderHeight]: INSET_HEADER_HEIGHT_DEFAULT,
} as const;

export const channelContentTopPaddingMeasurement = {
  cssVariable: chromeCssVars.channelContentTopPadding,
  resetValue: chromeCssVarDefaults[chromeCssVars.channelContentTopPadding],
} as const;

export const insetHeaderHeightMeasurement = {
  cssVariable: chromeCssVars.insetHeaderHeight,
  resetValue: chromeCssVarDefaults[chromeCssVars.insetHeaderHeight],
} as const;

/**
 * Tailwind class fragments for a flowed `TopChromeInsetHeader` that overlays
 * the scrollable content below it, so the content scrolls under the
 * translucent blurred header (same treatment as the channel header).
 */
export const insetHeaderOverlay = {
  /** Negative bottom margin pulling the next sibling under the header. */
  negativeMargin: "-mb-(--buzz-inset-header-height,5rem)",
  /** Padding-top reserving the measured header height inside the scroll area. */
  contentPadding: "pt-(--buzz-inset-header-height,5rem)",
  /**
   * Single full-width backdrop strip drawn behind transparent inset headers.
   * Rendered once per view so the blur samples continuously across column
   * boundaries instead of clipping at each pane's backdrop-filter box.
   * Carries the search-strip hairline (`before:`) and the bottom border.
   */
  backdrop:
    "pointer-events-none absolute inset-x-0 top-0 z-30 h-(--buzz-inset-header-height,5rem) border-b border-border/35 bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55 before:pointer-events-none before:absolute before:inset-x-0 before:top-(--buzz-top-chrome-height,2.5rem) before:h-px before:bg-border/35 before:content-['']",
} as const;

/** Tailwind class fragments for layout under the global top chrome. */
export const topChromeInset = {
  /** Absolute/fixed top offset below the search bar. */
  top: "top-(--buzz-top-chrome-height,2.5rem)",
  /** Padding-top clearing the global top chrome. */
  padding: "pt-(--buzz-top-chrome-height,2.5rem)",
  /** `after:` pseudo-element top offset. */
  afterTop: "after:top-(--buzz-top-chrome-height,2.5rem)",
  /** Horizontal divider at the bottom edge of the global top chrome inset. */
  divider:
    "before:pointer-events-none before:absolute before:inset-x-0 before:top-(--buzz-top-chrome-height,2.5rem) before:h-px before:bg-border/35 before:content-['']",
  /** Shared header backdrop and bottom border below the inset row. */
  headerBase:
    "relative z-40 shrink-0 border-b border-border/35 bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
  /** Vertical pane divider starting below the global top chrome. */
  verticalDivider:
    "after:pointer-events-none after:absolute after:bottom-0 after:right-0 after:top-(--buzz-top-chrome-height,2.5rem) after:z-40 after:w-px after:bg-border/35 after:content-['']",
} as const;

/** Tailwind class fragments for the global top chrome backdrop strip. */
export const topChromeBackdrop = {
  /** Height matching the global top chrome search/drag strip. */
  height: "h-(--buzz-top-chrome-height,2.5rem)",
  /** `after:` pseudo-element offset aligned to the bottom of top chrome. */
  dividerTop: "after:top-(--buzz-top-chrome-height,2.5rem)",
} as const;

/** Tailwind class fragments for measured channel header chrome. */
export const channelChrome = {
  /** Padding-top that clears the measured channel header chrome. */
  contentPadding: "pt-(--buzz-channel-content-top-padding,5.75rem)",
  /** Absolute/fixed top offset below the measured channel header chrome. */
  top: "top-(--buzz-channel-content-top-padding,5.75rem)",
  /** Height matching the measured channel header chrome. */
  headerHeight: "h-(--buzz-channel-content-top-padding,5.75rem)",
  /** Negative margin for overlaid channel chrome that should not affect flow. */
  negativeMargin: "-mb-(--buzz-channel-content-top-padding,5.75rem)",
} as const;
