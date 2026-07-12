import type { QueryClient, QueryKey } from "@tanstack/react-query";

export const HOUSEHOLD_FEATURE_ENABLED = import.meta.env.VITE_ENABLE_HOUSEHOLDS === "true";

export type HouseholdLensKind = "self" | "household" | "member" | "shared" | "unassigned";

export interface HouseholdLens {
  /** Opaque, server-issued selector. It is not a person or resource id. */
  selector: string;
  label: string;
  kind: HouseholdLensKind;
}

export interface HouseholdScope {
  /** Authenticated subject returned by the server; never supplied as authority to data APIs. */
  subject: string;
  lens: string;
}

export interface AuthorisedLenses {
  subject: string;
  selectedLens: string;
  lenses: HouseholdLens[];
}

interface FetchResponse {
  ok: boolean;
  json(): Promise<unknown>;
}

type FetchAuthorisedLenses = () => Promise<FetchResponse>;

const lensKinds = new Set<HouseholdLensKind>([
  "self",
  "household",
  "member",
  "shared",
  "unassigned",
]);

// ── Opaque selector validation (G0-H item 4) ──

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const NUMERIC_PATTERN = /^\d+$/;
const RESOURCE_ID_PATTERN = /^(acct|member|user|acc|account)_/i;

export function isOpaqueSelector(value: string): boolean {
  if (UUID_PATTERN.test(value)) return false;
  if (NUMERIC_PATTERN.test(value)) return false;
  if (RESOURCE_ID_PATTERN.test(value)) return false;
  return true;
}

// ── Central query key hash function (G0-H item 1) ──

let _globalScope: HouseholdScope = { subject: "local", lens: "self" };

/** Set by HouseholdScopeProvider so the QueryClient-level hash fn stays current. */
export function setGlobalHouseholdScope(scope: HouseholdScope): void {
  _globalScope = scope;
}

export function getGlobalHouseholdScope(): HouseholdScope {
  return _globalScope;
}

/**
 * Creates a queryKeyHashFn that incorporates subject+lens for cache isolation.
 * When subject is "local" (no household), hashes the original key unchanged.
 */
export function createSubjectScopedHashFn(scope: HouseholdScope): (key: QueryKey) => string {
  return (key: QueryKey) => {
    if (scope.subject === "local") {
      return JSON.stringify(key);
    }
    return JSON.stringify(["subject", scope.subject, "lens", scope.lens, ...key]);
  };
}

// ── Query key prefixing (G0-H item 1) ──

export function createHouseholdQueryKey(scope: HouseholdScope, key: QueryKey): QueryKey {
  if (scope.subject === "local") return key;
  return ["subject", scope.subject, "lens", scope.lens, ...key];
}

// ── Lenses response parser with opaque validation (G0-H item 4) ──

export function parseAuthorisedLenses(raw: {
  subject: string;
  selectedLens: string;
  lenses: { selector: string; label: string; kind: string }[];
}): AuthorisedLenses {
  const lenses = raw.lenses
    .filter((l): l is HouseholdLens => {
      if (
        typeof l.selector !== "string" ||
        typeof l.label !== "string" ||
        typeof l.kind !== "string"
      )
        return false;
      if (!lensKinds.has(l.kind as HouseholdLensKind)) return false;
      if (!isOpaqueSelector(l.selector)) return false;
      return true;
    })
    .map((l) => ({ selector: l.selector, label: l.label, kind: l.kind as HouseholdLensKind }));

  const selectedLens =
    typeof raw.selectedLens === "string" &&
    isOpaqueSelector(raw.selectedLens) &&
    lenses.some((lens) => lens.selector === raw.selectedLens)
      ? raw.selectedLens
      : (lenses.find((lens) => lens.kind === "self")?.selector ?? "self");

  return { subject: raw.subject, selectedLens, lenses };
}

export async function fetchAuthorisedLenses(
  request: FetchAuthorisedLenses = () =>
    fetch("/api/v1/household/lenses", { credentials: "same-origin" }),
): Promise<AuthorisedLenses> {
  const response = await request();
  if (!response.ok) throw new Error("Unable to load household lenses");

  const value = await response.json();
  if (!isRecord(value) || typeof value.subject !== "string" || !Array.isArray(value.lenses)) {
    throw new Error("Invalid household lens response");
  }

  return parseAuthorisedLenses(
    value as {
      subject: string;
      selectedLens: string;
      lenses: { selector: string; label: string; kind: string }[];
    },
  );
}

export function reconcileHouseholdScope(
  queryClient: QueryClient,
  previous: HouseholdScope | undefined,
  next: HouseholdScope,
): HouseholdScope {
  if (previous && previous.subject !== next.subject) queryClient.clear();
  return next;
}

export function handleHouseholdSecurityEvent(
  queryClient: QueryClient,
  scope: HouseholdScope,
  event: { type: string },
): HouseholdScope {
  if (event.type !== "household-access-revoked") return scope;
  queryClient.clear();
  return { ...scope, lens: "self" };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

// ── Generation fencing utilities (G0-H items 3 & 6) ──

/**
 * Check if a mutation completion is stale (fired after generation advanced).
 */
export function isStaleCompletion(genAtDispatch: number, currentGeneration: number): boolean {
  return genAtDispatch !== currentGeneration;
}

/**
 * Broadcast message with generation fencing payload (G0-H item 6).
 */
export interface HouseholdBroadcastMessage {
  type: "household-access-revoked" | "subject-generation";
  generation: number;
  subject: string;
}

/**
 * Check whether a received broadcast message is stale (older or equal to current gen).
 */
export function isStaleBroadcastMessage(
  message: HouseholdBroadcastMessage,
  currentGeneration: number,
): boolean {
  if (typeof message.generation !== "number") return true;
  return message.generation <= currentGeneration;
}

// ── Mutation abort tracking (G0-H items 3 & 7) ──

/**
 * Collection of AbortControllers for in-flight mutations.
 * Used to cancel active mutations on generation advance / logout.
 */
export function createMutationAbortRegistry() {
  const controllers: AbortController[] = [];

  return {
    /** Register a new AbortController for an in-flight mutation. */
    register(controller: AbortController): void {
      controllers.push(controller);
    },
    /** Unregister a completed/aborted AbortController. */
    unregister(controller: AbortController): void {
      const idx = controllers.indexOf(controller);
      if (idx >= 0) controllers.splice(idx, 1);
    },
    /** Abort ALL registered controllers and clear the list. */
    abortAll(): void {
      for (const c of controllers) {
        if (!c.signal.aborted) c.abort();
      }
      controllers.length = 0;
    },
    /** Number of tracked controllers. */
    get size(): number {
      return controllers.length;
    },
  };
}

// ── Tauri detection (G0-H item 8) ──

/**
 * Check if running inside a Tauri webview context.
 * Relies on navigator.userAgent containing "Tauri/" or "__TAURI__" global.
 */
export function isTauriContext(): boolean {
  if (typeof window === "undefined") return false;
  if (typeof (window as unknown as Record<string, unknown>).__TAURI__ !== "undefined") return true;
  if (typeof navigator !== "undefined" && /Tauri/i.test(navigator.userAgent)) return true;
  return false;
}
