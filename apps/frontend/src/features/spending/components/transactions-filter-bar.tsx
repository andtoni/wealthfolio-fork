import { useState } from "react";
import type { DateRange } from "react-day-picker";
import { useTranslation } from "react-i18next";

import {
  Button,
  FacetedFilter,
  Icons,
  ScrollArea,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@wealthfolio/ui";

import { cn } from "@/lib/utils";

import { AmountRangeFilter, type AmountRange } from "./amount-range-filter";
import { DateRangeFilter } from "./date-range-filter";
import type { CashActivityStatusFilter } from "../types/cash-activity";

export interface FilterOption {
  value: string;
  label: string;
}

interface TransactionsFilterBarProps {
  // Search
  searchInput: string;
  onSearchInputChange: (next: string) => void;

  // Filters
  statusFilter: CashActivityStatusFilter;
  onStatusFilterChange: (next: CashActivityStatusFilter) => void;
  dateRange: DateRange | undefined;
  onDateRangeChange: (next: DateRange | undefined) => void;
  selectedAccounts: Set<string>;
  onAccountsChange: (next: Set<string>) => void;
  selectedTypes: Set<string>;
  onTypesChange: (next: Set<string>) => void;
  selectedCategories: Set<string>;
  onCategoriesChange: (next: Set<string>) => void;
  selectedSubcategories: Set<string>;
  onSubcategoriesChange: (next: Set<string>) => void;
  selectedEvents: Set<string>;
  onEventsChange: (next: Set<string>) => void;
  amountRange: AmountRange;
  onAmountRangeChange: (next: AmountRange) => void;

  // Options
  accountOptions: FilterOption[];
  typeOptions: FilterOption[];
  categoryOptions: FilterOption[];
  subcategoryOptions: FilterOption[];
  eventOptions: FilterOption[];
  hasEvents: boolean;

  // Status
  filtersActive: boolean;
  onClearAll: () => void;

  // Count display
  isMobile?: boolean;
  /** Phone only: the page title and the select toggle shown beside the filter button. */
  title?: string;
  selectMode?: boolean;
  onToggleSelect?: () => void;
}

export function TransactionsFilterBar({
  searchInput,
  onSearchInputChange,
  statusFilter,
  onStatusFilterChange,
  dateRange,
  onDateRangeChange,
  selectedAccounts,
  onAccountsChange,
  selectedTypes,
  onTypesChange,
  selectedCategories,
  onCategoriesChange,
  selectedSubcategories,
  onSubcategoriesChange,
  selectedEvents,
  onEventsChange,
  amountRange,
  onAmountRangeChange,
  accountOptions,
  typeOptions,
  categoryOptions,
  subcategoryOptions,
  eventOptions,
  hasEvents,
  filtersActive,
  onClearAll,
  isMobile = false,
  title,
  selectMode = false,
  onToggleSelect,
}: TransactionsFilterBarProps) {
  const { t } = useTranslation();
  const [mobileFiltersOpen, setMobileFiltersOpen] = useState(false);
  const statusOptions = [
    { value: "needs_review", label: t("spending:filters.statusNeedsReview") },
    { value: "uncategorized", label: t("spending:filters.statusUncategorized") },
    { value: "categorized", label: t("spending:filters.statusCategorized") },
  ];
  const controlFiltersActive =
    statusFilter !== "all" ||
    selectedAccounts.size > 0 ||
    selectedTypes.size > 0 ||
    selectedCategories.size > 0 ||
    selectedSubcategories.size > 0 ||
    selectedEvents.size > 0 ||
    amountRange.min != null ||
    amountRange.max != null ||
    !!dateRange?.from ||
    !!dateRange?.to;
  const chips = [
    { label: t("spending:filters.category"), Icon: Icons.Tag, active: selectedCategories.size > 0 },
    { label: t("activity:table_account"), Icon: Icons.Wallet, active: selectedAccounts.size > 0 },
    {
      label: t("activity:table_date"),
      Icon: Icons.Calendar,
      active: !!dateRange?.from || !!dateRange?.to,
    },
    {
      label: t("spending:filters.statusUncategorized"),
      Icon: Icons.ListFilter,
      active: statusFilter !== "all",
    },
    { label: t("activity:table_type"), Icon: Icons.ArrowLeftRight, active: selectedTypes.size > 0 },
  ];
  const filterControls = (
    <>
      <FacetedFilter
        title={t("common:status")}
        options={statusOptions}
        selectedValues={new Set(statusFilter === "all" ? [] : [statusFilter])}
        onFilterChange={(v) => {
          const arr = Array.from(v);
          onStatusFilterChange((arr[0] as CashActivityStatusFilter) ?? "all");
        }}
      />
      <DateRangeFilter value={dateRange} onChange={onDateRangeChange} />
      <FacetedFilter
        title={t("common:account")}
        options={accountOptions}
        selectedValues={selectedAccounts}
        onFilterChange={onAccountsChange}
      />
      <FacetedFilter
        title={t("common:type")}
        options={typeOptions}
        selectedValues={selectedTypes}
        onFilterChange={onTypesChange}
      />
      <AmountRangeFilter value={amountRange} onChange={onAmountRangeChange} />
      <FacetedFilter
        title={t("spending:filters.category")}
        options={categoryOptions}
        selectedValues={selectedCategories}
        onFilterChange={onCategoriesChange}
      />
      <FacetedFilter
        title={t("spending:filters.subcategory")}
        options={subcategoryOptions}
        selectedValues={selectedSubcategories}
        onFilterChange={onSubcategoriesChange}
      />
      {hasEvents && (
        <FacetedFilter
          title={t("spending:filters.event")}
          options={eventOptions}
          selectedValues={selectedEvents}
          onFilterChange={onEventsChange}
        />
      )}
    </>
  );

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3 pt-1">
        <h1 className="text-[32px] font-medium leading-tight tracking-tight">{title}</h1>
        <div className="bg-card flex shrink-0 items-center rounded-full p-1 shadow-sm">
          {onToggleSelect && (
            <button
              type="button"
              className={cn(
                "flex h-9 w-10 items-center justify-center rounded-full",
                selectMode && "bg-foreground/10",
              )}
              onClick={onToggleSelect}
              aria-label={selectMode ? t("spending:filters.done") : t("common:select")}
              aria-pressed={selectMode}
            >
              <Icons.ListChecks className="h-[18px] w-[18px]" />
            </button>
          )}
          <button
            type="button"
            className="relative flex h-9 w-10 items-center justify-center rounded-full"
            onClick={() => setMobileFiltersOpen(true)}
            aria-label={t("spending:filters.filterTransactions")}
          >
            <Icons.ListFilter className="h-[18px] w-[18px]" />
            {controlFiltersActive && (
              <span className="bg-primary absolute right-2 top-1.5 h-2 w-2 rounded-full" />
            )}
          </button>
        </div>
      </div>
      <label className="bg-card flex h-11 items-center gap-2.5 rounded-full px-4 shadow-sm">
        <Icons.Search className="h-[18px] w-[18px] shrink-0" aria-hidden="true" />
        <input
          placeholder={t("common:search_placeholder")}
          value={searchInput}
          onChange={(e) => onSearchInputChange(e.target.value)}
          className="placeholder:text-muted-foreground h-full min-w-0 flex-1 bg-transparent text-[15px] outline-none"
        />
      </label>
      <div
        data-no-swipe-drag
        className="-mx-4 overflow-x-auto whitespace-nowrap px-4 [scrollbar-width:none] md:mx-0 md:px-0 [&::-webkit-scrollbar]:hidden"
      >
        {chips.map((chip) => (
          <button
            key={chip.label}
            type="button"
            className={cn(
              "mr-2 inline-flex h-9 items-center gap-1.5 rounded-full px-3.5 text-sm",
              chip.active ? "bg-foreground text-background" : "bg-foreground/[0.07]",
            )}
            onClick={() => setMobileFiltersOpen(true)}
          >
            <chip.Icon className="h-4 w-4" aria-hidden="true" />
            {chip.label}
          </button>
        ))}
      </div>
      <Sheet open={mobileFiltersOpen} onOpenChange={setMobileFiltersOpen}>
        <SheetContent
          side={isMobile ? "bottom" : "right"}
          className={cn("flex flex-col p-0", isMobile && "rounded-t-4xl mx-1 h-[80vh]")}
        >
          <SheetHeader className="border-border border-b px-6 py-4 text-left">
            <SheetTitle>{t("spending:filters.filterTransactions")}</SheetTitle>
          </SheetHeader>
          <ScrollArea className="flex-1">
            <div className="flex flex-wrap gap-2 px-6 py-4">{filterControls}</div>
          </ScrollArea>
          <SheetFooter className="border-border flex-row border-t px-6 py-4">
            <Button
              variant="ghost"
              className="text-muted-foreground hover:text-foreground"
              onClick={onClearAll}
              disabled={!filtersActive}
            >
              {t("spending:filters.clearAll")}
            </Button>
            <Button className="ml-auto" onClick={() => setMobileFiltersOpen(false)}>
              {t("spending:filters.done")}
            </Button>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </div>
  );
}
