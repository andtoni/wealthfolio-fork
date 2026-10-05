import type { ReactNode } from "react";

/** Section heading on the dashboard tabs, the size of a date heading in Transactions. */
export const SECTION_HEADING = "text-xl font-medium tracking-tight";

/** A list row without a box: a round avatar, two lines of text, an amount. */
export const LIST_ROW =
  "hover:bg-foreground/5 active:bg-foreground/5 -mx-2 flex max-w-none! items-center gap-3 rounded-xl px-2 py-2.5";

interface DashboardCardProps {
  title: string;
  /** Inline subtitle next to the title (e.g. "May 2026"). */
  subtitle?: ReactNode;
  /** Right-aligned slot — a toggle, link, etc. Takes precedence over `meta`. */
  action?: ReactNode;
  /** Convenience: right-aligned meta text (e.g. "vs prior 6M"). */
  meta?: string;
  /** Extra classes for the section body. */
  className?: string;
  children: ReactNode;
}

/**
 * A section on the main dashboard tabs: a heading (title + optional inline
 * subtitle + optional right-side action/meta) above its content, with no box.
 */
export function DashboardCard({
  title,
  subtitle,
  action,
  meta,
  className,
  children,
}: DashboardCardProps) {
  return (
    <div className="w-full min-w-0 max-w-full">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2 pb-2">
        <div className="flex min-w-0 flex-wrap items-baseline gap-2">
          <h2 className={SECTION_HEADING}>{title}</h2>
          {subtitle != null && subtitle !== "" && (
            <span className="text-muted-foreground text-sm">{subtitle}</span>
          )}
        </div>
        {action ? (
          <div className="min-w-0 max-w-full shrink-0">{action}</div>
        ) : meta ? (
          <span className="text-muted-foreground text-sm">{meta}</span>
        ) : null}
      </div>
      <div className={className}>{children}</div>
    </div>
  );
}
