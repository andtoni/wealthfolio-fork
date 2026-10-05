import { LIST_ROW } from "@/components/dashboard-card";
import { TickerAvatar } from "@/components/ticker-avatar";
import { SplitAmount } from "@/features/spending/components/split-amount";
import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import { HoldingType } from "@/lib/constants";
import { parseOccSymbol } from "@/lib/occ-symbol";
import { Account, AccountScope, Holding } from "@/lib/types";
import { cn } from "@/lib/utils";
import { AmountDisplay, GainPercent, Input } from "@wealthfolio/ui";
import { Button } from "@wealthfolio/ui/components/ui/button";
import { Icons } from "@wealthfolio/ui/components/ui/icons";
import { Skeleton } from "@wealthfolio/ui/components/ui/skeleton";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { HoldingsMobileFilterSheet } from "./holdings-mobile-filter-sheet";

type PerformanceMode = "daily" | "pnl" | "return";

interface HoldingsTableMobileProps {
  holdings: Holding[];
  isLoading: boolean;
  selectedTypes: string[];
  setSelectedTypes: (types: string[]) => void;
  accountFilter: AccountScope;
  onAccountScopeChange: (filter: AccountScope) => void;
  accounts: Account[];
  portfolios: { id: string; name: string }[];
  showAccountScope?: boolean;
  showSearch?: boolean;
  showFilterButton?: boolean;
  sortBy?: "symbol" | "marketValue" | "gain";
  setSortBy?: (value: "symbol" | "marketValue" | "gain") => void;
  performanceMode?: PerformanceMode;
  setPerformanceMode?: (value: PerformanceMode) => void;
  typeOptions?: { value: string; label: string }[];
}

export const HoldingsTableMobile = ({
  holdings,
  isLoading,
  selectedTypes,
  setSelectedTypes,
  accountFilter,
  onAccountScopeChange,
  accounts,
  portfolios,
  showAccountScope = true,
  showSearch = true,
  showFilterButton = true,
  sortBy: controlledSortBy,
  setSortBy: controlledSetSortBy,
  performanceMode: controlledPerformanceMode,
  setPerformanceMode: controlledSetPerformanceMode,
  typeOptions,
}: HoldingsTableMobileProps) => {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();
  const navigate = useNavigate();
  const [searchQuery, setSearchQuery] = useState("");
  const [isFilterSheetOpen, setIsFilterSheetOpen] = useState(false);

  // Internal state for uncontrolled mode
  const [internalSortBy, setInternalSortBy] = useState<"symbol" | "marketValue" | "gain">(
    "marketValue",
  );
  const [internalPerformanceMode, setInternalPerformanceMode] = useState<PerformanceMode>("pnl");

  const sortBy = controlledSortBy ?? internalSortBy;
  const setSortBy = controlledSetSortBy ?? setInternalSortBy;
  const performanceMode = controlledPerformanceMode ?? internalPerformanceMode;
  const setPerformanceMode = controlledSetPerformanceMode ?? setInternalPerformanceMode;

  const hasActiveFilters = useMemo(() => {
    const hasAccountScope = showAccountScope && accountFilter.type !== "all";
    const hasTypeFilter = selectedTypes.length > 0;
    return hasAccountScope || hasTypeFilter;
  }, [accountFilter, selectedTypes, showAccountScope]);

  const filteredHoldings = useMemo(() => {
    let result = [...holdings];

    if (selectedTypes.length > 0) {
      result = result.filter((holding) => {
        const assetType = holding.instrument?.classifications?.assetType?.name;
        return assetType && selectedTypes.includes(assetType);
      });
    }

    if (searchQuery) {
      const lowercasedQuery = searchQuery.toLowerCase();
      result = result.filter((holding) => {
        const nameMatch = holding.instrument?.name?.toLowerCase().includes(lowercasedQuery);
        const symbolMatch = holding.instrument?.symbol?.toLowerCase().includes(lowercasedQuery);

        return nameMatch || symbolMatch;
      });
    }

    return result.sort((a, b) => {
      if (sortBy === "marketValue") {
        const valA = a.marketValue?.base ?? 0;
        const valB = b.marketValue?.base ?? 0;
        return valB - valA; // Descending
      }
      if (sortBy === "gain") {
        const gain = (holding: Holding) =>
          performanceMode === "return"
            ? (holding.totalReturn?.base ?? holding.totalGain?.base ?? 0)
            : performanceMode === "pnl"
              ? (holding.totalGain?.base ?? 0)
              : (holding.dayChange?.base ?? 0);
        return gain(b) - gain(a);
      }

      const symbolA = a.instrument?.symbol?.toLowerCase() ?? "";
      const symbolB = b.instrument?.symbol?.toLowerCase() ?? "";
      if (symbolA && symbolB) {
        return symbolA.localeCompare(symbolB);
      }
      if (symbolA) {
        return -1;
      }
      if (symbolB) {
        return 1;
      }
      return 0;
    });
  }, [holdings, selectedTypes, searchQuery, sortBy, performanceMode]);

  const handleNavigate = (holding: Holding) => {
    // Use instrument.id (asset ID) for navigation, not symbol (which may be stripped)
    const assetId = holding.instrument?.id;
    if (assetId && !assetId.startsWith("$CASH")) {
      navigate(`/holdings/${encodeURIComponent(assetId)}`, { state: { holding } });
    }
  };

  if (isLoading) {
    return (
      <div className="space-y-2">
        <Skeleton className="h-20 w-full rounded-lg" />
        <Skeleton className="h-20 w-full rounded-lg" />
        <Skeleton className="h-20 w-full rounded-lg" />
        <Skeleton className="h-20 w-full rounded-lg" />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {(showSearch || showFilterButton) && (
        <div className="flex items-center gap-2">
          {showSearch && (
            <Input
              placeholder={t("holdings:search_placeholder")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="bg-secondary/30 h-10 flex-1 rounded-full border-none"
            />
          )}
          {showFilterButton && (
            <Button
              variant="outline"
              size="icon"
              className="relative size-9 shrink-0"
              onClick={() => setIsFilterSheetOpen(true)}
            >
              <Icons.ListFilter className="h-4 w-4" />
              {hasActiveFilters && (
                <span className="bg-destructive absolute right-0 top-0.5 h-2 w-2 rounded-full" />
              )}
            </Button>
          )}
        </div>
      )}
      <div>
        {filteredHoldings.length > 0 ? (
          filteredHoldings.map((holding) => {
            const symbol = holding.instrument?.symbol ?? holding.id;
            const isCash = holding.holdingType === HoldingType.CASH || symbol.startsWith("$CASH");
            const parsedOption = isCash ? null : parseOccSymbol(symbol);
            const avatarSymbol = isCash
              ? `CASH:${holding.localCurrency}`
              : parsedOption
                ? parsedOption.underlying
                : symbol;
            const displaySymbol = isCash
              ? symbol.split("-")[0]
              : parsedOption
                ? parsedOption.underlying
                : symbol;
            const subtitle = parsedOption
              ? `${new Date(parsedOption.expiration + "T12:00:00").toLocaleDateString("en-US", { month: "short", day: "numeric" })} $${parsedOption.strikePrice} ${parsedOption.optionType}`
              : (holding.instrument?.name ?? null);
            const isNavigable = !isCash && holding.instrument?.symbol;

            return (
              <div
                key={holding.id}
                className={cn(LIST_ROW, isNavigable && "cursor-pointer")}
                onClick={() => isNavigable && handleNavigate(holding)}
              >
                <TickerAvatar symbol={avatarSymbol} className="h-10 w-10 shrink-0" />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[17px] leading-snug">{displaySymbol}</p>
                  {subtitle && (
                    <p className="text-muted-foreground truncate text-[15px] leading-snug">
                      {subtitle}
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 flex-col items-end">
                  <span className="text-[19px] leading-snug">
                    <SplitAmount
                      value={holding.marketValue?.local ?? 0}
                      currency={holding.localCurrency}
                    />
                  </span>
                  <div className="flex items-center gap-2">
                    <AmountDisplay
                      value={
                        performanceMode === "return"
                          ? (holding.totalReturn?.local ?? holding.totalGain?.local ?? 0)
                          : performanceMode === "pnl"
                            ? (holding.totalGain?.local ?? 0)
                            : (holding.dayChange?.local ?? 0)
                      }
                      currency={holding.localCurrency}
                      isHidden={isBalanceHidden}
                      colorFormat
                      className="text-[13px]"
                    />
                    <GainPercent
                      value={
                        performanceMode === "return"
                          ? (holding.totalReturnPct ?? holding.totalGainPct ?? 0)
                          : performanceMode === "pnl"
                            ? (holding.totalGainPct ?? 0)
                            : (holding.dayChangePct ?? 0)
                      }
                      className="text-[13px]"
                    />
                  </div>
                </div>
              </div>
            );
          })
        ) : (
          <div className="flex h-48 flex-col items-center justify-center p-8 text-center">
            <h3 className="text-lg font-medium">{t("holdings:no_positions_found")}</h3>
            <p className="text-muted-foreground text-sm">
              {holdings.length === 0
                ? t("holdings:add_activities_prompt")
                : t("holdings:try_adjusting_filters")}
            </p>
          </div>
        )}
      </div>

      {/* Filter Sheet */}
      <HoldingsMobileFilterSheet
        open={isFilterSheetOpen}
        onOpenChange={setIsFilterSheetOpen}
        accountFilter={accountFilter}
        onAccountScopeChange={onAccountScopeChange}
        accounts={accounts}
        portfolios={portfolios}
        selectedTypes={selectedTypes}
        setSelectedTypes={setSelectedTypes}
        showAccountScope={showAccountScope}
        sortBy={sortBy}
        setSortBy={setSortBy}
        performanceMode={performanceMode}
        setPerformanceMode={setPerformanceMode}
        typeOptions={typeOptions}
      />
    </div>
  );
};

export default HoldingsTableMobile;
