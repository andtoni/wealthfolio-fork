import { SECTION_HEADING } from "@/components/dashboard-card";
import { getPlatforms } from "@/features/wealthfolio-connect";
import { useAccounts } from "@/hooks/use-accounts";
import { QueryKeys } from "@/lib/query-keys";
import type { Account, Platform } from "@/lib/types";
import { useQuery } from "@tanstack/react-query";
import {
  Button,
  EmptyPlaceholder,
  Icons,
  Skeleton,
  ToggleGroup,
  ToggleGroupItem,
} from "@wealthfolio/ui";
import { Input } from "@wealthfolio/ui/components/ui/input";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { SettingsHeader } from "../settings-header";
import { AccountEditModal } from "./components/account-edit-modal";
import { AccountItem } from "./components/account-item";
import { useAccountMutations } from "./components/use-account-mutations";

type FilterType = "all" | "active" | "archived" | "hidden";

const SettingsAccountsPage = () => {
  const { t } = useTranslation();
  const { accounts, isLoading } = useAccounts({ filterActive: false, includeArchived: true });
  const navigate = useNavigate();
  const location = useLocation();
  const [searchParams, setSearchParams] = useSearchParams();

  const { data: platforms } = useQuery<Platform[], Error>({
    queryKey: [QueryKeys.PLATFORMS],
    queryFn: getPlatforms,
  });

  // Create a map of platform ID to platform for quick lookup
  const platformMap = useMemo(() => {
    if (!platforms) return new Map<string, Platform>();
    return new Map(platforms.map((p) => [p.id, p]));
  }, [platforms]);

  const [visibleModal, setVisibleModal] = useState(false);
  const [selectedAccount, setSelectedAccount] = useState<Account | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [filter, setFilter] = useState<FilterType>("all");

  const handleAddAccount = () => navigate("/connect");

  useEffect(() => {
    const requestedByNavigation = (location.state as { newAccount?: boolean } | null)?.newAccount;
    if (searchParams.get("new") === "1" || requestedByNavigation) {
      setSelectedAccount(null);
      setVisibleModal(true);
      if (requestedByNavigation) {
        navigate(location.pathname, { replace: true, state: null });
      } else {
        setSearchParams({}, { replace: true });
      }
    }
  }, [location.pathname, location.state, navigate, searchParams, setSearchParams]);

  const { deleteAccountMutation, updateAccountMutation } = useAccountMutations({});

  const handleEditAccount = (account: Account) => {
    setSelectedAccount(account);
    setVisibleModal(true);
  };

  const handleDeleteAccount = (account: Account) => {
    deleteAccountMutation.mutate(account.id);
  };

  const handleArchiveAccount = (account: Account, archive: boolean) => {
    updateAccountMutation.mutate({
      ...account,
      isArchived: archive,
    });
  };

  const handleHideAccount = (account: Account, hide: boolean) => {
    updateAccountMutation.mutate({
      ...account,
      isActive: !hide,
    });
  };

  // Filter and search accounts
  const filteredAccounts = useMemo(() => {
    let result = accounts;

    // Apply search filter
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter(
        (account) =>
          account.name.toLowerCase().includes(query) ||
          account.group?.toLowerCase().includes(query) ||
          account.currency.toLowerCase().includes(query),
      );
    }

    // Apply status filter
    switch (filter) {
      case "active":
        result = result.filter((a) => a.isActive && !a.isArchived);
        break;
      case "archived":
        result = result.filter((a) => a.isArchived);
        break;
      case "hidden":
        result = result.filter((a) => !a.isActive && !a.isArchived);
        break;
      default:
        break;
    }

    return result;
  }, [accounts, searchQuery, filter]);

  // Split accounts into active and hidden/archived sections
  const { activeAccounts, inactiveAccounts } = useMemo(() => {
    const active = filteredAccounts.filter((a) => a.isActive && !a.isArchived);
    // Sort inactive: hidden first, then archived
    const inactive = filteredAccounts
      .filter((a) => !a.isActive || a.isArchived)
      .sort((a, b) => {
        // Hidden (not archived) comes before archived
        if (a.isArchived && !b.isArchived) return 1;
        if (!a.isArchived && b.isArchived) return -1;
        return 0;
      });
    return { activeAccounts: active, inactiveAccounts: inactive };
  }, [filteredAccounts]);

  // Counts for section headers
  const counts = useMemo(
    () => ({
      active: accounts.filter((a) => a.isActive && !a.isArchived).length,
      hidden: accounts.filter((a) => !a.isActive && !a.isArchived).length,
      archived: accounts.filter((a) => a.isArchived).length,
    }),
    [accounts],
  );

  if (isLoading) {
    return (
      <div>
        <Skeleton className="h-12" />
        <Skeleton className="h-12" />
      </div>
    );
  }

  const renderAccountItem = (account: Account) => (
    <AccountItem
      key={account.id}
      account={account}
      platform={account.platformId ? platformMap.get(account.platformId) : undefined}
      onEdit={handleEditAccount}
      onDelete={handleDeleteAccount}
      onArchive={handleArchiveAccount}
      onHide={handleHideAccount}
    />
  );

  const showSections = filter === "all";

  return (
    <>
      <div data-testid="settings-accounts-page" className="space-y-6">
        <SettingsHeader
          heading={t("settings:accounts_title")}
          text={t("settings:accounts_description")}
          actionsInline
        >
          {/* Mobile: icon button; Desktop: full button */}
          <>
            <Button
              data-testid="add-account-icon-button"
              size="icon"
              className="sm:hidden"
              onClick={() => handleAddAccount()}
              aria-label={t("settings:accounts_add_aria")}
            >
              <Icons.Plus className="h-4 w-4" />
            </Button>
            <Button
              data-testid="add-account-header-button"
              size="sm"
              className="hidden sm:inline-flex"
              onClick={() => handleAddAccount()}
            >
              <Icons.Plus className="mr-2 h-4 w-4" />
              {t("settings:accounts_add_button")}
            </Button>
          </>
        </SettingsHeader>

        {/* Search and Filter Bar */}
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="relative flex-1 sm:max-w-sm">
            <Icons.Search className="text-muted-foreground absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2" />
            <Input
              type="text"
              placeholder={t("settings:accounts.search_placeholder")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="bg-card !h-10 rounded-full border-0 pl-9 pr-9 text-[15px] shadow-sm"
            />
            {searchQuery && (
              <button
                type="button"
                onClick={() => setSearchQuery("")}
                className="text-muted-foreground hover:text-foreground absolute right-3 top-1/2 -translate-y-1/2"
                aria-label={t("settings:accounts.clear_search")}
              >
                <Icons.Close className="h-4 w-4" />
              </button>
            )}
          </div>

          <ToggleGroup
            type="single"
            value={filter}
            onValueChange={(value) => value && setFilter(value as FilterType)}
            className="bg-foreground/[0.07] h-9 rounded-full p-1"
          >
            <ToggleGroupItem
              value="all"
              className="data-[state=on]:bg-card h-7 rounded-full px-3 text-[13px] data-[state=on]:shadow-sm"
            >
              {t("settings:accounts.filter_all")}
            </ToggleGroupItem>
            <ToggleGroupItem
              value="active"
              className="data-[state=on]:bg-card h-7 rounded-full px-3 text-[13px] data-[state=on]:shadow-sm"
            >
              {t("settings:accounts.filter_active")}
            </ToggleGroupItem>
            <ToggleGroupItem
              value="hidden"
              className="data-[state=on]:bg-card h-7 rounded-full px-3 text-[13px] data-[state=on]:shadow-sm"
            >
              {t("settings:accounts.filter_hidden")}
            </ToggleGroupItem>
            <ToggleGroupItem
              value="archived"
              className="data-[state=on]:bg-card h-7 rounded-full px-3 text-[13px] data-[state=on]:shadow-sm"
            >
              {t("settings:accounts.filter_archived")}
            </ToggleGroupItem>
          </ToggleGroup>
        </div>

        {/* Account Lists */}
        <div className="space-y-8">
          {accounts.length === 0 ? (
            <EmptyPlaceholder>
              <EmptyPlaceholder.Icon name="Wallet" />
              <EmptyPlaceholder.Title>{t("settings:accounts_empty_title")}</EmptyPlaceholder.Title>
              <EmptyPlaceholder.Description>
                {t("settings:accounts_empty_description")}
              </EmptyPlaceholder.Description>
              <Button data-testid="add-account-empty-button" onClick={() => handleAddAccount()}>
                <Icons.Plus className="mr-2 h-4 w-4" />
                {t("settings:accounts_add_first")}
              </Button>
            </EmptyPlaceholder>
          ) : filteredAccounts.length === 0 ? (
            <div className="text-muted-foreground py-8 text-center">
              {t("settings:accounts.no_search_results")}
            </div>
          ) : showSections ? (
            <>
              {/* Active Accounts Section */}
              {activeAccounts.length > 0 && (
                <div className="space-y-3">
                  <div className="flex items-baseline gap-2">
                    <h3 className={SECTION_HEADING}>{t("settings:accounts.section_active")}</h3>
                    <span className="text-muted-foreground text-[15px] tabular-nums">
                      {counts.active}
                    </span>
                  </div>
                  <div className="">{activeAccounts.map(renderAccountItem)}</div>
                </div>
              )}

              {/* Hidden & Archived Section */}
              {inactiveAccounts.length > 0 && (
                <div className="space-y-3">
                  <div className="flex items-baseline gap-2">
                    <h3 className={SECTION_HEADING}>
                      {t("settings:accounts.section_hidden_archived")}
                    </h3>
                    <span className="text-muted-foreground text-[15px] tabular-nums">
                      {counts.hidden + counts.archived}
                    </span>
                  </div>
                  <div className="">{inactiveAccounts.map(renderAccountItem)}</div>
                </div>
              )}
            </>
          ) : (
            /* Flat list when filtered */
            <div className="">{filteredAccounts.map(renderAccountItem)}</div>
          )}
        </div>
      </div>
      <AccountEditModal
        account={selectedAccount || undefined}
        open={visibleModal}
        onClose={() => setVisibleModal(false)}
      />
    </>
  );
};

export default SettingsAccountsPage;
