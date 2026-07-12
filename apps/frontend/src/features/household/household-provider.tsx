/* eslint-disable react-refresh/only-export-components */
import { isWeb } from "@/adapters";
import { useAuth } from "@/context/auth-context";
import {
  useQuery,
  useQueryClient,
  type QueryKey,
  type UseQueryOptions,
  type UseQueryResult,
  type QueryFunction,
} from "@tanstack/react-query";
import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { HouseholdLensSelector } from "./household-lens-selector";
import {
  fetchAuthorisedLenses,
  HOUSEHOLD_FEATURE_ENABLED,
  setGlobalHouseholdScope,
  createHouseholdQueryKey,
  isStaleBroadcastMessage,
  isTauriContext,
  createMutationAbortRegistry,
  type HouseholdLens,
  type HouseholdScope,
  type HouseholdBroadcastMessage,
} from "./household-scope";

interface HouseholdContextValue extends HouseholdScope {
  lenses: HouseholdLens[];
  generation: number;
}
const disabled: HouseholdContextValue = {
  subject: "local",
  lens: "self",
  lenses: [],
  generation: 0,
};
const Context = createContext<HouseholdContextValue>(disabled);
export const useHouseholdScope = () => useContext(Context);

// Mutation abort registry shared across provider and auth logout
export const mutationRegistry = createMutationAbortRegistry();

async function purge(client: ReturnType<typeof useQueryClient>) {
  // Cancel in-flight mutations first
  mutationRegistry.abortAll();
  await client.cancelQueries();
  client.clear();
  client.getQueryCache().clear();
  client.getMutationCache().clear();
}

export function HouseholdScopeProvider({ children }: { children: React.ReactNode }) {
  const auth = useAuth();
  const client = useQueryClient();
  const active = isWeb && HOUSEHOLD_FEATURE_ENABLED && auth.isAuthenticated;
  const [value, setValue] = useState<HouseholdContextValue>(disabled);
  const current = useRef(value);
  current.current = value;
  const channel = useRef<BroadcastChannel | null>(null);

  // G0-H item 8: Tauri inert — skip event listeners in native context
  const isNative = !isWeb || isTauriContext();

  const revoke = useCallback(
    async (broadcast: boolean) => {
      await purge(client);
      setValue((v) => {
        const gen = v.generation + 1;
        const next = {
          ...v,
          lens: v.lenses.find((l) => l.kind === "self")?.selector ?? "self",
          generation: gen,
        };
        setGlobalHouseholdScope(next);
        return next;
      });
      // G0-H item 6: include generation + subject in broadcast messages
      if (broadcast && channel.current) {
        const msg: HouseholdBroadcastMessage = {
          type: "household-access-revoked",
          generation: value.generation + 1, // already incremented above
          subject: value.subject,
        };
        channel.current.postMessage(msg);
      }
    },
    [client, value],
  );

  useEffect(() => {
    if (!active || isNative) return;

    let disposed = false;
    const loadScope = () => {
      void fetchAuthorisedLenses().then(async (data) => {
        if (disposed) return;
        if (current.current.subject !== "local" && current.current.subject !== data.subject)
          await purge(client);
        setValue((v) => {
          const gen = v.generation + 1;
          const next: HouseholdContextValue = {
            subject: data.subject,
            lens: data.selectedLens,
            lenses: data.lenses,
            generation: gen,
          };
          setGlobalHouseholdScope(next);
          return next;
        });
      });
    };
    loadScope();

    window.addEventListener("focus", loadScope);

    const bc =
      typeof BroadcastChannel === "undefined"
        ? null
        : new BroadcastChannel("wealthfolio-household-security");
    channel.current = bc;
    if (bc) {
      bc.onmessage = (event: MessageEvent) => {
        const msg = event.data as HouseholdBroadcastMessage;
        if (!msg || !msg.type) return;
        // G0-H item 6: generation fencing for broadcast messages
        if (msg.type === "household-access-revoked" || msg.type === "subject-generation") {
          if (isStaleBroadcastMessage(msg, current.current.generation)) {
            return; // discard stale/reordered events
          }
          void revoke(false);
        }
      };
    }

    const events = new EventSource("/api/v1/events", { withCredentials: true });
    events.addEventListener("household-access-revoked", () => void revoke(true));
    events.addEventListener("subject-generation", () => void revoke(true));

    return () => {
      disposed = true;
      window.removeEventListener("focus", loadScope);
      events.close();
      bc?.close();
      channel.current = null;
    };
  }, [active, client, revoke, isNative]);

  const handleSelect = useCallback(
    (lens: string) => {
      if (!value.lenses.some((l) => l.selector === lens)) return;
      void purge(client).then(() =>
        setValue((v) => {
          const gen = v.generation + 1;
          const next: HouseholdContextValue = { ...v, lens, generation: gen };
          setGlobalHouseholdScope(next);
          return next;
        }),
      );
    },
    [client, value.lenses],
  );

  setGlobalHouseholdScope(value);

  if (!active || isNative) return <Context.Provider value={disabled}>{children}</Context.Provider>;
  return (
    <Context.Provider value={value}>
      <div className="flex justify-end px-4 pt-2">
        <HouseholdLensSelector
          enabled
          selected={value.lens}
          lenses={value.lenses}
          onSelect={handleSelect}
        />
      </div>
      {children}
    </Context.Provider>
  );
}

// ── useFinancialQuery – wrapper that scopes keys and injects lens header (G0-H items 1 & 2) ──

/**
 * Wraps useQuery with household scope:
 * 1. Prefixes queryKey with subject+lens
 * 2. Injects lens as header/param via queryFn meta
 */
export function useFinancialQuery<TData = unknown, TError = unknown>(
  options: UseQueryOptions<TData, TError> & { queryKey: QueryKey },
): UseQueryResult<TData, TError> {
  const scope = useHouseholdScope();
  const scopedKey = createHouseholdQueryKey(scope, options.queryKey);

  const enhancedFn: QueryFunction<TData, QueryKey> | undefined = options.queryFn
    ? (ctx) => {
        const meta = { ...ctx.meta, lens: scope.lens, subject: scope.subject };
        return (options.queryFn as QueryFunction<TData, QueryKey>)({ ...ctx, meta });
      }
    : undefined;

  return useQuery<TData, TError>({
    ...options,
    queryKey: scopedKey,
    queryFn: enhancedFn,
  });
}
