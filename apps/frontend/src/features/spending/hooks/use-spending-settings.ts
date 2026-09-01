import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { QueryKeys } from "@/lib/query-keys";

import { getSpendingSettings, updateSpendingSettings } from "../adapters/settings";
import { invalidateSpendingCaches } from "../lib/invalidation";
import type { SpendingSettings, SpendingSettingsUpdate } from "../types";

export function useSpendingSettings() {
  const query = useQuery<SpendingSettings, Error>({
    queryKey: [QueryKeys.SPENDING_SETTINGS],
    queryFn: getSpendingSettings,
    staleTime: 60_000,
  });

  return {
    settings: query.data,
    isEnabled: query.data?.enabled ?? false,
    accountIds: query.data?.accountIds ?? [],
    budgetCycle: query.data?.budgetCycle ?? {
      cadence: "monthly" as const,
      anchorDate: "2026-07-25",
    },
    isLoading: query.isLoading,
    error: query.error,
  };
}

export function useSpendingSettingsMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (update: SpendingSettingsUpdate) => updateSpendingSettings(update),
    onSuccess: (data) => {
      queryClient.setQueryData([QueryKeys.SPENDING_SETTINGS], data);
      // Account opt-in changes the universe of cash activities — invalidate every
      // spending-scoped cache so each view refetches with the new account_ids.
      invalidateSpendingCaches(queryClient);
      queryClient.invalidateQueries({ queryKey: [QueryKeys.ACTIVITY_DATA] });
      toast.success("Spending settings updated.");
    },
    onError: (error) => {
      const message =
        error instanceof Error ? error.message : typeof error === "string" ? error : undefined;
      toast.error(message || "Failed to update spending settings.");
    },
  });
}
