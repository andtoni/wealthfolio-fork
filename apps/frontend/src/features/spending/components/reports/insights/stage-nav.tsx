import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

export type InsightsStage = "where" | "changed" | "when";

interface StageNavProps {
  stage: InsightsStage;
  onStageChange: (s: InsightsStage) => void;
}

export function StageNav({ stage, onStageChange }: StageNavProps) {
  const { t } = useTranslation();
  const STAGES = useMemo<{ id: InsightsStage; label: string }[]>(
    () => [
      { id: "where", label: t("spending:insights.stageWhere") },
      { id: "changed", label: t("spending:insights.stageChanged") },
      { id: "when", label: t("spending:insights.stageWhen") },
    ],
    [t],
  );
  const activeRef = useRef<HTMLButtonElement | null>(null);
  // When the URL deep-links to a stage on mount or `stage` changes
  // (e.g. via the dashboard "Where I am" link), scroll the active chip
  // into view so it isn't off-screen on a 375px column.
  useEffect(() => {
    activeRef.current?.scrollIntoView({
      behavior: "smooth",
      block: "nearest",
      inline: "center",
    });
  }, [stage]);

  return (
    <nav
      aria-label={t("spending:insights.stagesLabel")}
      className="bg-foreground/[0.07] flex w-full items-center gap-1 rounded-full p-1 sm:inline-flex sm:w-auto"
    >
      {STAGES.map((s) => {
        const active = stage === s.id;
        return (
          <button
            key={s.id}
            type="button"
            ref={active ? activeRef : undefined}
            onClick={() => onStageChange(s.id)}
            className={cn(
              "group inline-flex flex-1 items-center justify-center gap-2 whitespace-nowrap rounded-full px-2 py-2 text-[13px] transition-colors sm:px-4 sm:text-sm",
              active
                ? "bg-card text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
            aria-current={active ? "step" : undefined}
          >
            <span className="font-medium">{s.label}</span>
          </button>
        );
      })}
    </nav>
  );
}
