import { isWeb } from "@/adapters";
import { AddonRuntimeLoader } from "@/addons/addon-runtime-loader";
import { setAddonQueryClient } from "@/addons/addons-runtime-context";
import { Toaster } from "@/components/sonner";
import { AuthGate, AuthProvider } from "@/context/auth-context";
import { EventDialogProvider } from "@/features/spending/components/event-dialog-provider";
import { WealthfolioConnectProvider } from "@/features/wealthfolio-connect";
import { HouseholdScopeProvider } from "@/features/household/household-provider";
import {
  createSubjectScopedHashFn,
  getGlobalHouseholdScope,
} from "@/features/household/household-scope";
import { SettingsProvider } from "@/lib/settings-provider";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@wealthfolio/ui";
import { useState } from "react";
import { PrivacyProvider } from "./context/privacy-context";
import { LoginPage } from "./pages/auth/login-page";
import { AppRoutes } from "./routes";

function App() {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            refetchOnWindowFocus: false,
            staleTime: 5 * 60 * 1000,
            retry: false,
            // G0-H item 1: subject-scoped query key hashing for cache isolation
            queryKeyHashFn: createSubjectScopedHashFn(getGlobalHouseholdScope()),
          },
        },
      }),
  );

  const isWebEnv = isWeb;

  setAddonQueryClient(queryClient as unknown as Parameters<typeof setAddonQueryClient>[0]);

  const routedContent = isWebEnv ? (
    <AuthGate fallback={<LoginPage />}>
      <AppRoutes />
    </AuthGate>
  ) : (
    <AppRoutes />
  );

  return (
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <HouseholdScopeProvider>
          <WealthfolioConnectProvider>
            <PrivacyProvider>
              <SettingsProvider>
                <TooltipProvider>
                  <Toaster mobileOffset={{ top: "68px" }} closeButton expand={false} />
                  <AddonRuntimeLoader />
                  <EventDialogProvider>{routedContent}</EventDialogProvider>
                </TooltipProvider>
              </SettingsProvider>
            </PrivacyProvider>
          </WealthfolioConnectProvider>
        </HouseholdScopeProvider>
      </AuthProvider>
    </QueryClientProvider>
  );
}

export default App;
