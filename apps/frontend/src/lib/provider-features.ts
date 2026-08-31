import { isWeb } from "@/adapters";
import { invoke } from "@/adapters/shared/platform";
import type { AppInfo } from "@/adapters/types";
import { useQuery } from "@tanstack/react-query";

export function useInvestecCardLifecycleLinkEnabled(): boolean {
  const { data } = useQuery({
    queryKey: ["app-info"],
    queryFn: () => invoke<AppInfo>("get_app_info"),
    enabled: isWeb,
    retry: 3,
    staleTime: Number.POSITIVE_INFINITY,
  });

  return isWeb && data?.investecCardLifecycleLinkEnabled === true;
}
