import type * as React from "react";

import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

type TopChromeInsetHeaderProps = React.ComponentProps<"div"> & {
  /**
   * Render without its own backdrop, borders, and hairlines — for headers
   * stacked on a shared `insetHeaderOverlay.backdrop` strip that spans
   * multiple columns (keeps the blur continuous across pane boundaries).
   */
  transparent?: boolean;
};

/**
 * Flowed header row that clears the global search/drag chrome and draws the
 * horizontal separator at the bottom edge of that inset.
 */
export function TopChromeInsetHeader({
  className,
  children,
  transparent = false,
  ...props
}: TopChromeInsetHeaderProps) {
  return (
    <div
      className={cn(
        transparent
          ? "relative z-40 shrink-0"
          : cn(topChromeInset.headerBase, topChromeInset.divider),
        topChromeInset.padding,
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}
