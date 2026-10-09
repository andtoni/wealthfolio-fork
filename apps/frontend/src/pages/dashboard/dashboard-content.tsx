import { calculatePerformanceSummary } from "@/adapters";
import { ChangeLine } from "@/components/change-line";
import { HistoryChart } from "@/components/history-chart";
import { useHapticFeedback } from "@/hooks";
import { useCurrentValuation } from "@/hooks/use-current-account-valuations";
import { useHoldings } from "@/hooks/use-holdings";
import { useValuationHistory } from "@/hooks/use-valuation-history";
import { HoldingType, isAlternativeAssetKind } from "@/lib/constants";
import { pageWash } from "@/lib/page-wash";
import {
  changeBetween,
  performancePeriodPnl,
  performanceSummaryReturn,
  type Change,
} from "@/lib/performance";
import { QueryKeys } from "@/lib/query-keys";
import { useSettingsContext } from "@/lib/settings-provider";
import type { AccountValuation, DateRange, PerformanceResult, TimePeriod } from "@/lib/types";
import { PortfolioUpdateTrigger } from "@/pages/dashboard/portfolio-update-trigger";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import type { TimePeriod as UITimePeriod } from "@wealthfolio/ui";
import { getInitialIntervalData, IntervalSelector, usePersistentState } from "@wealthfolio/ui";
import { Skeleton } from "@wealthfolio/ui/components/ui/skeleton";
import { format, startOfMonth, subDays } from "date-fns";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AccountsSummary } from "./accounts-summary";
import Balance from "./balance";
import SavingGoals from "./goals";
import TopHoldings from "./top-holdings";

const DEFAULT_INTERVAL: UITimePeriod = "3M";
const INTERVAL_STORAGE_KEY = "dashboard-interval";

function getDashboardChartMinDomainSpanRatio(period: UITimePeriod): number {
  switch (period) {
    case "1D":
    case "1W":
      return 0.035;
    case "1M":
    case "3M":
      return 0.08;
    case "6M":
    case "YTD":
    case "1Y":
      return 0.16;
    case "5Y":
    case "ALL":
      return 0.2;
    default:
      return 0.12;
  }
}

function getDashboardNetContributionMaxDomainSpanRatio(period: UITimePeriod): number | undefined {
  switch (period) {
    case "1D":
    case "1W":
      return undefined;
    case "1M":
    case "3M":
      return 1.4;
    case "6M":
    case "YTD":
    case "1Y":
      return 2.2;
    case "5Y":
    case "ALL":
      return 2.8;
    default:
      return 1.8;
  }
}

const portfolioPerformanceQuery = (startDate?: string, endDate?: string) => ({
  queryKey: [QueryKeys.PERFORMANCE_SUMMARY, "dashboard", "all", startDate, endDate],
  queryFn: () =>
    calculatePerformanceSummary({
      itemType: "account",
      itemId: "portfolio:all",
      startDate,
      endDate,
      filter: { type: "all" },
      profile: "dashboard",
    }),
  placeholderData: keepPreviousData,
  staleTime: 30 * 1000,
  retry: 1,
});

// Investment gain when performance can be measured; otherwise (cash-only
// accounts, incomplete valuations) the plain change in balance.
function periodChange(
  performance: PerformanceResult | undefined,
  history: AccountValuation[] | undefined,
): Change | null {
  const gain = performancePeriodPnl(performance);
  if (gain != null) return { amount: gain, percent: performanceSummaryReturn(performance) };
  if (!history || history.length < 2) return null;
  return changeBetween(history[0].totalValueBase, history[history.length - 1].totalValueBase);
}

export function DashboardContent() {
  const { t } = useTranslation();
  // Use the same persisted state as IntervalSelector for the interval code
  const [intervalCode] = usePersistentState<UITimePeriod>(INTERVAL_STORAGE_KEY, DEFAULT_INTERVAL);

  // Derive initial values from the persisted interval code
  const [dateRange, setDateRange] = useState<DateRange | undefined>(
    () => getInitialIntervalData(intervalCode).range,
  );
  const [selectedInterval, setSelectedInterval] = useState<UITimePeriod>(() => intervalCode);
  const [isAllTime, setIsAllTime] = useState<boolean>(() => intervalCode === "ALL");

  const { holdings: allHoldings, isLoading: isHoldingsLoading } = useHoldings({ type: "all" });
  const {
    currentValuation: portfolioCurrentValuation,
    isLoading: isCurrentValuationLoading,
    error: currentValuationError,
  } = useCurrentValuation({ type: "all" }, { includeAccounts: true });
  const { triggerHaptic } = useHapticFeedback();

  // Filter holdings for display (exclude alternative assets and cash for TopHoldings)
  const holdings = useMemo(() => {
    if (!allHoldings) return [];
    return allHoldings.filter((h) => {
      // Exclude cash holdings from display
      if (h.holdingType === HoldingType.CASH) return false;
      // Exclude alternative assets from display
      if (h.assetKind && isAlternativeAssetKind(h.assetKind)) return false;
      return true;
    });
  }, [allHoldings]);

  const totalValue = portfolioCurrentValuation?.summary.totalValueBase ?? 0;

  const valuationHistoryRange = isAllTime ? undefined : dateRange;
  const { valuationHistory, isLoading: isValuationHistoryLoading } =
    useValuationHistory(valuationHistoryRange);

  const { settings } = useSettingsContext();
  const baseCurrency = settings?.baseCurrency ?? "USD";

  const startDate =
    !isAllTime && dateRange?.from ? format(dateRange.from, "yyyy-MM-dd") : undefined;
  const endDate = !isAllTime && dateRange?.to ? format(dateRange.to, "yyyy-MM-dd") : undefined;
  const datesReady = isAllTime || (!!startDate && !!endDate);

  const { data: portfolioPerformance, isLoading: isPortfolioPerformanceLoading } = useQuery({
    ...portfolioPerformanceQuery(startDate, endDate),
    enabled: datesReady,
  });

  // "This month" runs from last month's closing value to today.
  const monthRange = useMemo(() => {
    const today = new Date();
    return { from: subDays(startOfMonth(today), 1), to: today };
  }, []);
  const { valuationHistory: monthHistory } = useValuationHistory(monthRange);
  const { data: monthPerformance } = useQuery(
    portfolioPerformanceQuery(
      format(monthRange.from, "yyyy-MM-dd"),
      format(monthRange.to, "yyyy-MM-dd"),
    ),
  );

  const isCurrentValuationUnavailable =
    !isCurrentValuationLoading && !portfolioCurrentValuation && Boolean(currentValuationError);
  const portfolioSourceDataAsOf =
    portfolioCurrentValuation?.summary.sourceDataAsOf ??
    (!isCurrentValuationUnavailable
      ? valuationHistory?.[valuationHistory.length - 1]?.calculatedAt
      : undefined);

  const chartData = useMemo(() => {
    return (
      valuationHistory?.map((item) => ({
        date: item.valuationDate,
        totalValue: item.totalValueBase,
        netContribution: item.netContributionBase,
        currency: item.baseCurrency ?? baseCurrency,
      })) ?? []
    );
  }, [valuationHistory, baseCurrency]);

  const chartMinDomainSpanRatio = useMemo(
    () => getDashboardChartMinDomainSpanRatio(selectedInterval),
    [selectedInterval],
  );
  const chartNetContributionMaxDomainSpanRatio = useMemo(
    () => getDashboardNetContributionMaxDomainSpanRatio(selectedInterval),
    [selectedInterval],
  );

  const rangeChange = periodChange(portfolioPerformance, valuationHistory);
  const monthChange = periodChange(monthPerformance, monthHistory);

  const isNegative = totalValue < 0;

  // Callback for IntervalSelector
  const handleIntervalSelect = (
    code: TimePeriod,
    _description: string,
    range: DateRange | undefined,
  ) => {
    setSelectedInterval(code);
    setDateRange(range);
    setIsAllTime(code === "ALL");
  };

  return (
    <div className="flex min-h-full flex-col">
      <div className="px-4 pb-1 pt-2 md:px-6 lg:px-8">
        <PortfolioUpdateTrigger
          lastCalculatedAt={portfolioSourceDataAsOf}
          notices={portfolioCurrentValuation?.summary.warnings}
        >
          <div className="flex items-start gap-2">
            <div>
              <Balance
                isLoading={isCurrentValuationLoading}
                isUnavailable={isCurrentValuationUnavailable}
                targetValue={totalValue}
                currency={baseCurrency}
                displayCurrency={true}
              />
              {isPortfolioPerformanceLoading ? (
                <Skeleton className="h-4 w-40" />
              ) : (
                <>
                  {monthChange && (
                    <ChangeLine
                      className="lg:text-md text-sm font-light"
                      change={monthChange}
                      currency={baseCurrency}
                      label={t("ui:change.this_month")}
                    />
                  )}
                  {rangeChange && (
                    <ChangeLine
                      className="text-xs font-light lg:text-sm"
                      change={rangeChange}
                      currency={baseCurrency}
                      label={t(`ui:interval.${selectedInterval}`)}
                      muted
                    />
                  )}
                </>
              )}
            </div>
          </div>
        </PortfolioUpdateTrigger>
      </div>

      <div
        className="flex grow flex-col"
        style={{
          backgroundImage: pageWash(isNegative ? "var(--destructive)" : "var(--success)"),
        }}
      >
        <div className="h-70">
          <HistoryChart
            data={chartData}
            isLoading={isValuationHistoryLoading}
            scaleMode="fit-visible"
            minDomainSpanRatio={chartMinDomainSpanRatio}
            netContributionMaxDomainSpanRatio={chartNetContributionMaxDomainSpanRatio}
          />
          {valuationHistory && chartData.length > 0 && (
            <div className="flex w-full justify-center">
              <IntervalSelector
                className="pointer-events-auto relative z-20 w-full max-w-screen-sm sm:max-w-screen-md md:max-w-2xl lg:max-w-3xl"
                onIntervalSelect={handleIntervalSelect}
                onHaptic={triggerHaptic}
                isLoading={isValuationHistoryLoading}
                storageKey={INTERVAL_STORAGE_KEY}
                defaultValue={DEFAULT_INTERVAL}
              />
            </div>
          )}
        </div>

        <div className="grow px-4 pb-[var(--mobile-nav-total-offset)] pt-14 md:px-6 md:pb-6 md:pt-12 lg:px-10 lg:pb-8 lg:pt-14">
          <div className="grid grid-cols-1 gap-8 lg:grid-cols-3 lg:gap-20">
            <div className="lg:col-span-2">
              <AccountsSummary
                dateRange={dateRange}
                isAllTime={isAllTime}
                currentAccountValuations={portfolioCurrentValuation?.accounts}
                isLoadingCurrentValuations={isCurrentValuationLoading}
              />
            </div>
            <div className="space-y-6 lg:col-span-1">
              <TopHoldings
                holdings={holdings}
                isLoading={isHoldingsLoading}
                baseCurrency={baseCurrency}
              />
              <SavingGoals />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default DashboardContent;
