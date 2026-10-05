import { LIST_ROW } from "@/components/dashboard-card";
import { SplitAmount } from "@/features/spending/components/split-amount";
import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import type { AlternativeAssetHolding } from "@/lib/types";
import { ALTERNATIVE_ASSET_KIND_DISPLAY_NAMES } from "@/lib/types";
import { AmountDisplay, GainPercent } from "@wealthfolio/ui";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@wealthfolio/ui/components/ui/alert-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@wealthfolio/ui/components/ui/dropdown-menu";
import { Icons } from "@wealthfolio/ui/components/ui/icons";
import { Skeleton } from "@wealthfolio/ui/components/ui/skeleton";
import { useState } from "react";
import { Trans, useTranslation } from "react-i18next";

interface AlternativeHoldingsListProps {
  holdings: AlternativeAssetHolding[];
  isLoading: boolean;
  onEdit: (holding: AlternativeAssetHolding) => void;
  onUpdateValue: (holding: AlternativeAssetHolding) => void;
  onViewHistory: (holding: AlternativeAssetHolding) => void;
  onDelete: (holding: AlternativeAssetHolding) => void;
  onRowClick: (holding: AlternativeAssetHolding) => void;
  isDeleting: boolean;
}

/** Personal assets or liabilities as plain rows, largest first, each with its own menu. */
export function AlternativeHoldingsList({
  holdings,
  isLoading,
  onEdit,
  onUpdateValue,
  onViewHistory,
  onDelete,
  onRowClick,
  isDeleting,
}: AlternativeHoldingsListProps) {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();
  const [assetToDelete, setAssetToDelete] = useState<AlternativeAssetHolding | null>(null);

  if (isLoading) {
    return (
      <div className="space-y-3 py-2.5">
        <Skeleton className="h-10 w-full rounded-lg" />
        <Skeleton className="h-10 w-full rounded-lg" />
        <Skeleton className="h-10 w-full rounded-lg" />
      </div>
    );
  }

  if (holdings.length === 0) {
    return null;
  }

  const sorted = [...holdings].sort(
    (a, b) => parseFloat(b.marketValue) - parseFloat(a.marketValue),
  );

  return (
    <div>
      {sorted.map((holding) => {
        const kindDisplay =
          ALTERNATIVE_ASSET_KIND_DISPLAY_NAMES[
            holding.kind.toUpperCase() as keyof typeof ALTERNATIVE_ASSET_KIND_DISPLAY_NAMES
          ] ?? holding.kind;

        const gain = holding.unrealizedGain ? parseFloat(holding.unrealizedGain) : null;
        const gainPct = holding.unrealizedGainPct ? parseFloat(holding.unrealizedGainPct) : null;

        return (
          <div key={holding.id} className={LIST_ROW}>
            <div
              role="button"
              tabIndex={0}
              className="flex min-w-0 flex-1 cursor-pointer items-center gap-3"
              onClick={() => onRowClick(holding)}
              onKeyDown={(event) => event.key === "Enter" && onRowClick(holding)}
            >
              <div className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
                <AssetKindIcon kind={holding.kind} size={20} />
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate text-[17px] leading-snug">{holding.name}</p>
                <p className="text-muted-foreground truncate text-[15px] leading-snug">
                  {kindDisplay}
                </p>
              </div>
              <div className="flex shrink-0 flex-col items-end">
                <span className="text-[19px] leading-snug">
                  <SplitAmount
                    value={parseFloat(holding.marketValue)}
                    currency={holding.currency}
                  />
                </span>
                {gain !== null && gainPct !== null && (
                  <div className="flex items-center gap-2">
                    {/* The amount needs the room a phone gives to the name. */}
                    <span className="hidden sm:inline">
                      <AmountDisplay
                        value={gain}
                        currency={holding.currency}
                        isHidden={isBalanceHidden}
                        colorFormat
                        className="text-[13px]"
                      />
                    </span>
                    <GainPercent value={gainPct} className="text-[13px]" />
                  </div>
                )}
              </div>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  className="hover:bg-foreground/5 text-muted-foreground flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors"
                  aria-label={t("holdings:open_actions")}
                >
                  <Icons.MoreVertical className="h-4 w-4" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => onUpdateValue(holding)}>
                  <Icons.DollarSign className="mr-2 h-4 w-4" />
                  {t("holdings:update_value")}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => onViewHistory(holding)}>
                  <Icons.History className="mr-2 h-4 w-4" />
                  {t("holdings:value_history")}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => onEdit(holding)}>
                  <Icons.Pencil className="mr-2 h-4 w-4" />
                  {t("holdings:edit_details")}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  onSelect={() => setAssetToDelete(holding)}
                >
                  <Icons.Trash className="mr-2 h-4 w-4" />
                  {t("common:delete")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        );
      })}

      <AlertDialog
        open={assetToDelete !== null}
        onOpenChange={(open) => !open && setAssetToDelete(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("holdings:delete_asset")}</AlertDialogTitle>
            <AlertDialogDescription>
              <Trans
                i18nKey="holdings:delete_asset_confirm"
                values={{ name: assetToDelete?.name }}
                components={{ bold: <span className="font-medium" /> }}
              />
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={isDeleting}>{t("common:cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (assetToDelete) onDelete(assetToDelete);
                setAssetToDelete(null);
              }}
              disabled={isDeleting}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {isDeleting ? (
                <>
                  <Icons.Spinner className="mr-2 h-4 w-4 animate-spin" />
                  {t("holdings:deleting")}
                </>
              ) : (
                t("common:delete")
              )}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function AssetKindIcon({ kind, size = 20 }: { kind: string; size?: number }) {
  switch (kind.toLowerCase()) {
    case "property":
      return <Icons.RealEstateDuotone size={size} />;
    case "vehicle":
      return <Icons.VehicleDuotone size={size} />;
    case "collectible":
      return <Icons.CollectibleDuotone size={size} />;
    case "precious":
      return <Icons.PreciousDuotone size={size} />;
    case "liability":
      return <Icons.LiabilityDuotone size={size} />;
    default:
      return <Icons.OtherAssetDuotone size={size} />;
  }
}
