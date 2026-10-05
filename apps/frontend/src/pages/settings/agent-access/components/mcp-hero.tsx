import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

/** Shared "hero" banner for AI Agent Access, used on both desktop and web. */
export function McpHero({
  active,
  title,
  description,
  hint,
  action,
}: {
  /** Whether the feature is on (drives the status dot + pulse). */
  active: boolean;
  /** Status line, e.g. "AI Agent Access · off". */
  title: string;
  /** Primary sentence. */
  description: string;
  /** Optional secondary line (e.g. how to enable). */
  hint?: ReactNode;
  /** Optional right-slot control (the enable toggle on desktop). */
  action?: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <section aria-label={t("settings:agentAccess.hero_aria")}>
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-[17px] leading-snug">
            <span
              className={cn(
                "h-2 w-2 shrink-0 rounded-full",
                active ? "bg-success" : "bg-foreground/25",
              )}
            />
            <span className="truncate">{title}</span>
          </div>
          <div className="text-muted-foreground text-[15px] leading-snug">{description}</div>
        </div>
        {action}
      </div>
      {hint && <div className="text-muted-foreground mt-2 text-[13px]">{hint}</div>}
    </section>
  );
}
