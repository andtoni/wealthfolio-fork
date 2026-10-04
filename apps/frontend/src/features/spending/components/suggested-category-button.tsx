import { Icons } from "@wealthfolio/ui";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

export interface SuggestedCategory {
  taxonomyId: string;
  id: string;
  name: string;
  color: string | null;
}

interface SuggestedCategoryButtonProps {
  category: SuggestedCategory;
  onAccept: () => void;
  className?: string;
}

/** One tap applies the category this merchant was given before. */
export function SuggestedCategoryButton({
  category,
  onAccept,
  className,
}: SuggestedCategoryButtonProps) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      aria-label={t("spending:transactions.useSuggestedCategory", { name: category.name })}
      className={cn("border-dashed", className)}
      onClick={onAccept}
    >
      {category.color && (
        <span
          className="h-2.5 w-2.5 shrink-0 rounded-full"
          style={{ backgroundColor: category.color }}
          aria-hidden="true"
        />
      )}
      <span className="truncate">{category.name}</span>
      <Icons.Check className="text-muted-foreground h-3 w-3 shrink-0" aria-hidden="true" />
    </button>
  );
}
