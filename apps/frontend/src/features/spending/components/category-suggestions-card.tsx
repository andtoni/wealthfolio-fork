import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { Icons } from "@wealthfolio/ui";

import { cn } from "@/lib/utils";

import { MerchantAvatar } from "./merchant-avatar";
import { QuickCategorizePopover, type QuickCategorizeScope } from "./quick-categorize-popover";
import { SplitAmount } from "./split-amount";
import type { SuggestedCategory } from "./suggested-category-button";

export type SuggestionCertainty = "sure" | "likely" | "guess";

/** Sure: several earlier labels, four in five agreeing. Likely: every earlier label agrees. */
export function suggestionCertainty(agreeing: number, total: number): SuggestionCertainty {
  if (total >= 2 && agreeing * 5 >= total * 4) return "sure";
  return agreeing === total ? "likely" : "guess";
}

export interface CategorySuggestionItem {
  activityId: string;
  name: string | null;
  amount: number;
  currency: string;
  sign: string;
  scope: QuickCategorizeScope;
  category: SuggestedCategory;
  certainty: SuggestionCertainty;
}

const BARS: Record<SuggestionCertainty, number> = { sure: 3, likely: 2, guess: 1 };

interface CategorySuggestionsCardProps {
  /** Suggestions for the transactions on screen. */
  items: CategorySuggestionItem[];
  /** Every suggestion there is, including transactions further down the list. */
  total: number;
  sureCount: number;
  onAccept: (activityId: string, taxonomyId: string, categoryId: string) => void;
  onAcceptSure: () => void;
  onClose: () => void;
}

/** Categories offered for uncategorized transactions, accepted here without leaving the list. */
export function CategorySuggestionsCard({
  items,
  total,
  sureCount,
  onAccept,
  onAcceptSure,
  onClose,
}: CategorySuggestionsCardProps) {
  const { t } = useTranslation();
  return (
    <div className="bg-card rounded-3xl p-4 shadow-sm">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-[17px] leading-snug">
            {t("spending:suggestions.title", { count: total })}
          </div>
          <div className="text-muted-foreground text-[13px]">
            {t("spending:suggestions.subtitle")}
          </div>
        </div>
        {sureCount > 0 ? (
          <button
            type="button"
            className="bg-foreground text-background h-9 shrink-0 rounded-full px-4 text-sm"
            onClick={onAcceptSure}
          >
            {t("spending:suggestions.acceptSure", { count: sureCount })}
          </button>
        ) : (
          <button
            type="button"
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full"
            onClick={onClose}
            aria-label={t("common:close")}
          >
            <Icons.Close className="h-[18px] w-[18px]" />
          </button>
        )}
      </div>
      {items.map((item) => (
        <div key={item.activityId} className="flex items-center gap-3 pt-3">
          <MerchantAvatar
            name={item.name}
            currency={item.currency}
            logo
            tint={item.category.color}
          />
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-baseline gap-2">
              <span className="truncate text-[15px]">{item.name ?? "—"}</span>
              <span className="text-muted-foreground shrink-0 text-[13px]">
                <SplitAmount value={item.amount} currency={item.currency} sign={item.sign} />
              </span>
            </div>
            <div className="mt-0.5 flex items-center gap-2">
              <QuickCategorizePopover
                scope={item.scope}
                selectedCategoryId={item.category.id}
                onSelect={(taxonomyId, categoryId) =>
                  onAccept(item.activityId, taxonomyId, categoryId)
                }
                trigger={
                  <button
                    type="button"
                    aria-label={t("spending:transactions.changeCategory", {
                      name: item.category.name,
                    })}
                    className="bg-foreground/[0.07] inline-flex min-w-0 items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[13px]"
                  >
                    <span
                      className="h-2 w-2 shrink-0 rounded-full"
                      style={{ backgroundColor: item.category.color ?? "var(--muted-foreground)" }}
                      aria-hidden="true"
                    />
                    <span className="truncate">{item.category.name}</span>
                  </button>
                }
              />
              <span className="flex shrink-0 items-center gap-0.5" aria-hidden="true">
                {[1, 2, 3].map((bar) => (
                  <span
                    key={bar}
                    className={cn(
                      "h-2.5 w-1 rounded-sm",
                      bar > BARS[item.certainty]
                        ? "bg-foreground/15"
                        : item.certainty === "sure"
                          ? "bg-success"
                          : "bg-[#C28B47]",
                    )}
                  />
                ))}
              </span>
              <span className="text-muted-foreground shrink-0 text-xs">
                {t(`spending:suggestions.${item.certainty}`)}
              </span>
            </div>
          </div>
          <button
            type="button"
            className={cn(
              "flex h-9 w-9 shrink-0 items-center justify-center rounded-full",
              item.certainty === "sure" ? "bg-foreground text-background" : "bg-foreground/[0.07]",
            )}
            onClick={() => onAccept(item.activityId, item.category.taxonomyId, item.category.id)}
            aria-label={t("spending:transactions.useSuggestedCategory", {
              name: item.category.name,
            })}
          >
            <Icons.Check className="h-4 w-4" />
          </button>
        </div>
      ))}
      <div className="text-muted-foreground flex flex-wrap items-center justify-between gap-x-3 gap-y-1 pt-3 text-[13px]">
        <span>
          {total > items.length
            ? t("spending:suggestions.more", { count: total - items.length })
            : total === 0
              ? t("spending:suggestions.none")
              : null}
        </span>
        <Link
          to="/assistant"
          state={{ aiPrompt: t("spending:tabContent.aiCategorizePrompt") }}
          className="text-foreground inline-flex items-center gap-1.5 underline-offset-4 hover:underline"
        >
          <Icons.Sparkles className="h-3.5 w-3.5" aria-hidden="true" />
          {t("spending:suggestions.askAssistant")}
        </Link>
      </div>
    </div>
  );
}
