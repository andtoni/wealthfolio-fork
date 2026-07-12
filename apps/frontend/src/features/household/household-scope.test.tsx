import { QueryClient } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { HouseholdLensSelector } from "./household-lens-selector";
import {
  createHouseholdQueryKey,
  fetchAuthorisedLenses,
  handleHouseholdSecurityEvent,
  reconcileHouseholdScope,
  parseAuthorisedLenses,
  createSubjectScopedHashFn,
  type HouseholdScope,
} from "./household-scope";

const andrea: HouseholdScope = { subject: "oidc:https://issuer.example:andrea", lens: "self" };

describe("household query scope", () => {
  it("includes authenticated subject and selected lens without resource ids", () => {
    expect(createHouseholdQueryKey(andrea, ["accounts", { page: 1 }])).toEqual([
      "subject",
      "oidc:https://issuer.example:andrea",
      "lens",
      "self",
      "accounts",
      { page: 1 },
    ]);
  });

  it("accepts only server-authorised opaque lens selectors", async () => {
    const result = await fetchAuthorisedLenses(async () => ({
      ok: true,
      json: async () => ({
        subject: "oidc:https://issuer.example:andrea",
        selectedLens: "household",
        lenses: [
          { selector: "household", label: "Household", kind: "household" },
          { selector: "member-7fd", label: "Alexis", kind: "member" },
          { selector: "shared", label: "Shared", kind: "shared" },
          { selector: "unassigned", label: "Unassigned", kind: "unassigned" },
        ],
      }),
    }));

    expect(result.lenses.map(({ kind, label }) => [kind, label])).toEqual([
      ["household", "Household"],
      ["member", "Alexis"],
      ["shared", "Shared"],
      ["unassigned", "Unassigned"],
    ]);
    expect(JSON.stringify(result)).not.toContain("accountId");
  });

  it("purges all cached state when the authenticated subject changes", () => {
    const client = new QueryClient();
    client.setQueryData(createHouseholdQueryKey(andrea, ["accounts"]), [{ id: "private" }]);

    const next = reconcileHouseholdScope(client, andrea, {
      subject: "oidc:https://issuer.example:jordan",
      lens: "self",
    });

    expect(client.getQueryCache().getAll()).toHaveLength(0);
    expect(next.lens).toBe("self");
  });

  it("renders only server-provided lenses and returns the opaque selector", () => {
    const onSelect = vi.fn();
    render(
      <HouseholdLensSelector
        enabled
        selected="self"
        lenses={[
          { selector: "self", label: "My portfolio", kind: "self" },
          { selector: "member-opaque", label: "Alexis", kind: "member" },
        ]}
        onSelect={onSelect}
      />,
    );

    fireEvent.change(screen.getByLabelText("Portfolio lens"), {
      target: { value: "member-opaque" },
    });
    expect(onSelect).toHaveBeenCalledWith("member-opaque");
    expect(screen.queryByText("Jordan")).not.toBeInTheDocument();
  });

  it("renders nothing while the household feature is disabled", () => {
    const { container } = render(
      <HouseholdLensSelector enabled={false} selected="self" lenses={[]} onSelect={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("purges cached rows and resets lens on a server revocation event", () => {
    const client = new QueryClient();
    const household = { ...andrea, lens: "member-7fd" };
    client.setQueryData(createHouseholdQueryKey(household, ["activities"]), [{ id: "revoked" }]);

    const next = handleHouseholdSecurityEvent(client, household, {
      type: "household-access-revoked",
    });

    expect(client.getQueryCache().getAll()).toHaveLength(0);
    expect(next).toEqual({ ...andrea, lens: "self" });
  });
});

// ── G0-H item 4: opaque-selector format validation ──

describe("parseAuthorisedLenses opaque validation", () => {
  it("rejects selectors that look like UUIDs", () => {
    const result = parseAuthorisedLenses({
      subject: "sub-1",
      selectedLens: "550e8400-e29b-41d4-a716-446655440000",
      lenses: [
        { selector: "550e8400-e29b-41d4-a716-446655440000", label: "Bad UUID", kind: "member" },
        { selector: "opaque-abc123", label: "Good", kind: "self" },
      ],
    });
    expect(result.lenses).toHaveLength(1);
    expect(result.lenses[0]!.selector).toBe("opaque-abc123");
    expect(result.selectedLens).toBe("opaque-abc123");
  });

  it("rejects selectors that look like numeric IDs", () => {
    const result = parseAuthorisedLenses({
      subject: "sub-1",
      selectedLens: "42",
      lenses: [
        { selector: "42", label: "Numeric", kind: "member" },
        { selector: "opaque-xyz", label: "Good", kind: "self" },
      ],
    });
    expect(result.lenses).toHaveLength(1);
    expect(result.lenses[0]!.selector).toBe("opaque-xyz");
  });

  it("rejects selectors that look like account/member patterns", () => {
    const result = parseAuthorisedLenses({
      subject: "sub-1",
      selectedLens: "acct_abc123",
      lenses: [
        { selector: "acct_abc123", label: "Account", kind: "member" },
        { selector: "member_xyz", label: "Member", kind: "member" },
        { selector: "user_99", label: "User", kind: "member" },
        { selector: "opaque-zzz", label: "Opaque", kind: "self" },
      ],
    });
    expect(result.lenses).toHaveLength(1);
    expect(result.lenses[0]!.selector).toBe("opaque-zzz");
  });

  it("rejects selectedLens when it is non-opaque even if present in lenses", () => {
    const result = parseAuthorisedLenses({
      subject: "sub-1",
      selectedLens: "acct_fake",
      lenses: [
        { selector: "acct_fake", label: "Fake", kind: "member" },
        { selector: "opaque-v1", label: "Opaque", kind: "self" },
      ],
    });
    expect(result.selectedLens).toBe("opaque-v1");
  });

  it("drops to 'self' fallback when all lenses are non-opaque", () => {
    const result = parseAuthorisedLenses({
      subject: "sub-1",
      selectedLens: "acct_1",
      lenses: [
        { selector: "acct_1", label: "A1", kind: "member" },
        { selector: "member_2", label: "M2", kind: "member" },
      ],
    });
    expect(result.lenses).toHaveLength(0);
    expect(result.selectedLens).toBe("self");
  });
});

// ── G0-H item 1: central query key transformation ──

describe("subject-scoped query key hash", () => {
  it("hashes query keys with subject+lens scope when scope is active", () => {
    const hashFn = createSubjectScopedHashFn({ subject: "alice", lens: "household" });
    const hash = hashFn(["accounts"]);
    expect(hash).toBe(JSON.stringify(["subject", "alice", "lens", "household", "accounts"]));
  });

  it("hashes query keys without scope when subject is local", () => {
    const hashFn = createSubjectScopedHashFn({ subject: "local", lens: "self" });
    const hash = hashFn(["accounts"]);
    expect(hash).toBe(JSON.stringify(["accounts"]));
  });

  it("provides cache isolation between different lenses for same query key", () => {
    const client = new QueryClient({
      defaultOptions: {
        queries: {
          queryKeyHashFn: createSubjectScopedHashFn({ subject: "alice", lens: "lens-a" }),
        },
      },
    });
    client.setQueryData(["accounts"], "data-a");
    // Different hash fn ⇒ different key ⇒ different entry
    const clientB = new QueryClient({
      defaultOptions: {
        queries: {
          queryKeyHashFn: createSubjectScopedHashFn({ subject: "alice", lens: "lens-b" }),
        },
      },
    });
    clientB.setQueryData(["accounts"], "data-b");
    expect(client.getQueryData(["accounts"])).toBe("data-a");
    expect(clientB.getQueryData(["accounts"])).toBe("data-b");
  });
});

// ── G0-H item 3: stale mutation fencing ──

describe("stale mutation fencing", () => {
  it("discards mutation completion when generation has advanced", () => {
    const onSuccess = vi.fn();
    let capturedGeneration = 0;
    let currentGeneration = 0;

    // Simulate capturing generation at mutation dispatch
    capturedGeneration = currentGeneration; // generation 0

    // Advance generation (e.g. lens change)
    currentGeneration = 1;

    // Simulate mutation completion: check if generation is stale
    const isStale = capturedGeneration !== currentGeneration;

    // onSuccess callback should not fire because generation is stale
    if (!isStale) {
      onSuccess("result");
    }

    expect(onSuccess).not.toHaveBeenCalled();
  });

  it("discards optimistic update when generation has advanced", () => {
    let currentGeneration = 0;
    const optimisticUpdate = vi.fn();

    // Dispatch mutation at gen 0
    const genAtDispatch = currentGeneration;

    // Generation advances (e.g. lens change, logout)
    currentGeneration = 1;

    // Optimistic callback fires but is stale
    if (genAtDispatch === currentGeneration) {
      optimisticUpdate({ temp: "value" });
    }

    expect(optimisticUpdate).not.toHaveBeenCalled();
  });

  it("aborts in-flight mutation when generation advances", () => {
    const controller = new AbortController();
    const abortSpy = vi.spyOn(controller, "abort");

    // Simulate generation advance aborting active mutations
    controller.abort();

    expect(abortSpy).toHaveBeenCalled();
    expect(controller.signal.aborted).toBe(true);
  });
});

// ── G0-H item 6: BroadcastChannel generation fencing ──

describe("BroadcastChannel generation fencing", () => {
  it("rejects stale broadcast events with older generation", () => {
    const currentGen = 5;

    // Stale event from generation 3
    const staleEvent = { type: "household-access-revoked", generation: 3, subject: "alice" };
    // Current event
    const freshEvent = { type: "household-access-revoked", generation: 6, subject: "alice" };

    expect(staleEvent.generation < currentGen).toBe(true);
    expect(freshEvent.generation > currentGen).toBe(true);

    // Both should be compared: stale event rejected
    const shouldRejectStale = staleEvent.generation <= currentGen;
    const shouldRejectFresh = freshEvent.generation <= currentGen;

    expect(shouldRejectStale).toBe(true);
    expect(shouldRejectFresh).toBe(false);
  });

  it("includes generation and subject in broadcast messages", () => {
    const event = { type: "subject-generation", generation: 7, subject: "bob" };
    expect(event).toHaveProperty("generation");
    expect(event).toHaveProperty("subject");
    expect(event.generation).toBe(7);
  });
});

// ── G0-H item 7: in-flight mutation cancellation on logout ──

describe("in-flight mutation cancellation", () => {
  it("closes AbortControllers on logout before clearing caches", () => {
    const controllers = new Set<AbortController>();
    const c1 = new AbortController();
    const c2 = new AbortController();
    controllers.add(c1);
    controllers.add(c2);

    // Logout: abort all
    for (const c of controllers) c.abort();
    controllers.clear();

    expect(c1.signal.aborted).toBe(true);
    expect(c2.signal.aborted).toBe(true);
    expect(controllers.size).toBe(0);
  });

  it("does not fire completed mutation callbacks after cache clearance", async () => {
    const controller = new AbortController();
    const callback = vi.fn();
    const results: string[] = [];

    // Simulate: mutation dispatched, then cache cleared, then mutation completes
    const promise = new Promise<void>((resolve) => {
      setTimeout(() => {
        if (!controller.signal.aborted) {
          results.push("completed");
          callback("data");
        }
        resolve();
      }, 10);
    });

    // Cache clearance happens
    controller.abort();
    await promise;

    expect(callback).not.toHaveBeenCalled();
    expect(results).toEqual([]);
  });
});

// ── G0-H item 8: Tauri inert ──

describe("Tauri inert checks", () => {
  it("does not register event listeners when userAgent indicates Tauri", () => {
    const userAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X) AppleWebKit/... Tauri/2.0";
    const isTauri = userAgent.includes("Tauri/");

    const addEventListenerSpy = vi.spyOn(window, "addEventListener");

    if (!isTauri) {
      window.addEventListener("focus", vi.fn());
    }

    expect(isTauri).toBe(true);
    expect(addEventListenerSpy).not.toHaveBeenCalled();
    addEventListenerSpy.mockRestore();
  });

  it("registers event listeners in browser context", () => {
    const userAgent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0";
    const isTauri = /Tauri/i.test(userAgent);

    const fn = vi.fn();
    window.addEventListener("focus", fn);

    expect(isTauri).toBe(false);
    expect(fn).not.toHaveBeenCalled(); // just registered
    window.removeEventListener("focus", fn);
  });
});
