import { ActivityType } from "@/lib/constants";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AccountSelectOption } from "../components/forms/fields";
import { ACTIVITY_FORM_CONFIG, type ActivityFormValues } from "../config/activity-form-config";
import { useActivityForm } from "./use-activity-form";

const mutationMocks = vi.hoisted(() => ({
  addMutateAsync: vi.fn(),
  updateMutateAsync: vi.fn(),
  saveMutateAsync: vi.fn(),
  savePairMutateAsync: vi.fn(),
  unlinkMutateAsync: vi.fn(),
}));

const adapterMocks = vi.hoisted(() => ({
  getTransferPairForActivity: vi.fn(),
  loggerError: vi.fn(),
}));

vi.mock("@/adapters", () => ({
  getTransferPairForActivity: adapterMocks.getTransferPairForActivity,
  logger: {
    error: adapterMocks.loggerError,
  },
}));

vi.mock("./use-activity-mutations", () => ({
  useActivityMutations: () => ({
    addActivityMutation: {
      mutateAsync: mutationMocks.addMutateAsync,
      isPending: false,
      error: null,
      isError: false,
    },
    updateActivityMutation: {
      mutateAsync: mutationMocks.updateMutateAsync,
      isPending: false,
      error: null,
      isError: false,
    },
    saveActivitiesMutation: {
      mutateAsync: mutationMocks.saveMutateAsync,
      isPending: false,
      error: null,
      isError: false,
    },
    saveInternalTransferPairMutation: {
      mutateAsync: mutationMocks.savePairMutateAsync,
      isPending: false,
      error: null,
      isError: false,
    },
    unlinkTransferActivitiesMutation: {
      mutateAsync: mutationMocks.unlinkMutateAsync,
      isPending: false,
      error: null,
      isError: false,
    },
  }),
}));

const accounts: AccountSelectOption[] = [
  { value: "acc-usd", label: "USD Account", currency: "USD" },
  { value: "acc-cad", label: "CAD Account", currency: "CAD" },
];

describe("useActivityForm", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mutationMocks.addMutateAsync.mockResolvedValue({});
    mutationMocks.updateMutateAsync.mockResolvedValue({});
    mutationMocks.saveMutateAsync.mockResolvedValue({});
    mutationMocks.savePairMutateAsync.mockResolvedValue({});
    mutationMocks.unlinkMutateAsync.mockResolvedValue({});
    adapterMocks.getTransferPairForActivity.mockReset();
    adapterMocks.loggerError.mockReset();
  });

  it("opens an existing non-transfer row as an unlinked transfer", () => {
    const defaults = ACTIVITY_FORM_CONFIG.TRANSFER.getDefaults(
      {
        id: "withdrawal-id",
        activityType: ActivityType.WITHDRAWAL,
        accountId: "acc-usd",
        amount: "-250",
      },
      accounts,
    );

    expect(defaults).toMatchObject({
      isExternal: false,
      direction: "out",
      accountId: "",
      fromAccountId: "acc-usd",
      toAccountId: "",
    });
  });

  it("keeps an existing internal transfer in paired mode", () => {
    const defaults = ACTIVITY_FORM_CONFIG.TRANSFER.getDefaults(
      {
        id: "transfer-out-id",
        activityType: ActivityType.TRANSFER_OUT,
        accountId: "acc-usd",
      },
      accounts,
    );

    expect(defaults).toMatchObject({
      isExternal: false,
      direction: "out",
      fromAccountId: "acc-usd",
    });
  });

  it("preserves incoming direction for an external transfer", () => {
    const defaults = ACTIVITY_FORM_CONFIG.TRANSFER.getDefaults(
      {
        id: "deposit-id",
        activityType: ActivityType.TRANSFER_IN,
        accountId: "acc-usd",
        metadata: { flow: { is_external: true } },
      },
      accounts,
    );

    expect(defaults).toMatchObject({
      isExternal: true,
      direction: "in",
      accountId: "acc-usd",
    });
  });

  it("converts a marked row through the external transfer update path", async () => {
    const { result } = renderHook(() =>
      useActivityForm({
        accounts,
        selectedType: "TRANSFER",
        activity: {
          id: "withdrawal-id",
          activityType: ActivityType.WITHDRAWAL,
          accountId: "acc-usd",
          amount: "-250",
        },
      }),
    );

    await act(async () => {
      await result.current.handleSubmit({
        ...result.current.defaultValues,
        isExternal: true,
        direction: "out",
        accountId: "acc-usd",
        amount: 250,
        transferMode: "cash",
        currency: "USD",
      } as ActivityFormValues);
    });

    expect(mutationMocks.updateMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "withdrawal-id",
        activityType: ActivityType.TRANSFER_OUT,
        metadata: { flow: { is_external: true } },
      }),
    );
  });

  it("preserves user-selected currency for DEPOSIT", async () => {
    const { result } = renderHook(() =>
      useActivityForm({
        accounts,
        selectedType: "DEPOSIT",
      }),
    );

    const formData = {
      accountId: "acc-usd",
      activityDate: new Date("2026-02-01T10:00:00.000Z"),
      amount: 1000,
      comment: "test",
      currency: "EUR",
      fxRate: 1.25,
    } as ActivityFormValues;

    await act(async () => {
      await result.current.handleSubmit(formData);
    });

    expect(mutationMocks.addMutateAsync).toHaveBeenCalledTimes(1);
    expect(mutationMocks.addMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: "acc-usd",
        activityType: ActivityType.DEPOSIT,
        currency: "EUR",
      }),
    );
  });

  it("falls back to account currency when DEPOSIT currency is empty", async () => {
    const { result } = renderHook(() =>
      useActivityForm({
        accounts,
        selectedType: "DEPOSIT",
      }),
    );

    const formData = {
      accountId: "acc-usd",
      activityDate: new Date("2026-02-01T10:00:00.000Z"),
      amount: 1000,
      comment: null,
      currency: "   ",
    } as ActivityFormValues;

    await act(async () => {
      await result.current.handleSubmit(formData);
    });

    expect(mutationMocks.addMutateAsync).toHaveBeenCalledTimes(1);
    expect(mutationMocks.addMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: "acc-usd",
        activityType: ActivityType.DEPOSIT,
        currency: "USD",
      }),
    );
  });

  it("preserves user-selected currency for external TRANSFER", async () => {
    const { result } = renderHook(() =>
      useActivityForm({
        accounts,
        selectedType: "TRANSFER",
      }),
    );

    const formData = {
      isExternal: true,
      direction: "in",
      accountId: "acc-usd",
      fromAccountId: "",
      toAccountId: "",
      activityDate: new Date("2026-02-01T10:00:00.000Z"),
      transferMode: "cash",
      amount: 250,
      assetId: null,
      quantity: null,
      unitPrice: null,
      comment: "external transfer",
      currency: "EUR",
      fxRate: 1.2,
      subtype: null,
      quoteMode: "MARKET",
    } as ActivityFormValues;

    await act(async () => {
      await result.current.handleSubmit(formData);
    });

    expect(mutationMocks.addMutateAsync).toHaveBeenCalledTimes(1);
    expect(mutationMocks.addMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: "acc-usd",
        activityType: ActivityType.TRANSFER_IN,
        currency: "EUR",
      }),
    );
  });

  it("updates both existing legs when editing an internal securities transfer", async () => {
    const { result } = renderHook(() =>
      useActivityForm({
        accounts,
        selectedType: "TRANSFER",
        activity: {
          id: "transfer-out-id",
          activityType: ActivityType.TRANSFER_OUT,
          transferOutId: "transfer-out-id",
          transferInId: "transfer-in-id",
        },
      }),
    );

    const formData = {
      isExternal: false,
      direction: "out",
      accountId: "",
      fromAccountId: "acc-usd",
      toAccountId: "acc-cad",
      activityDate: new Date("2026-02-01T10:00:00.000Z"),
      transferMode: "securities",
      amount: undefined,
      sourceAmount: undefined,
      destinationAmount: undefined,
      sourceCurrency: "USD",
      destinationCurrency: "CAD",
      assetId: "AAPL",
      quantity: 10,
      unitPrice: 100,
      comment: "move shares",
      currency: "USD",
      fxRate: 1.35,
      subtype: null,
      quoteMode: "MARKET",
    } as ActivityFormValues;

    await act(async () => {
      await result.current.handleSubmit(formData);
    });

    expect(mutationMocks.saveMutateAsync).toHaveBeenCalledTimes(1);
    expect(mutationMocks.saveMutateAsync).toHaveBeenCalledWith({
      updates: [
        expect.objectContaining({
          id: "transfer-out-id",
          accountId: "acc-usd",
          activityType: ActivityType.TRANSFER_OUT,
          currency: "USD",
        }),
        expect.objectContaining({
          id: "transfer-in-id",
          accountId: "acc-cad",
          activityType: ActivityType.TRANSFER_IN,
          currency: "CAD",
          fxRate: 1.35,
        }),
      ],
    });
  });

  it("unlinks a valid grouped transfer before saving it as external", async () => {
    adapterMocks.getTransferPairForActivity.mockResolvedValue({
      transferOut: { id: "transfer-out-id" },
      transferIn: { id: "transfer-in-id" },
    });

    const { result } = renderHook(() =>
      useActivityForm({
        accounts,
        selectedType: "TRANSFER",
        activity: {
          id: "transfer-in-id",
          activityType: ActivityType.TRANSFER_IN,
          accountId: "acc-cad",
          sourceGroupId: "group-1",
        },
      }),
    );

    const formData = {
      isExternal: true,
      direction: "in",
      accountId: "acc-cad",
      fromAccountId: "",
      toAccountId: "",
      activityDate: new Date("2026-02-01T10:00:00.000Z"),
      transferMode: "cash",
      amount: 250,
      assetId: null,
      quantity: null,
      unitPrice: null,
      comment: "external transfer",
      currency: "CAD",
      fxRate: null,
      subtype: null,
      quoteMode: "MARKET",
    } as ActivityFormValues;

    await act(async () => {
      await result.current.handleSubmit(formData);
    });

    expect(adapterMocks.getTransferPairForActivity).toHaveBeenCalledWith("transfer-in-id");
    expect(mutationMocks.unlinkMutateAsync).toHaveBeenCalledWith({
      activityAId: "transfer-out-id",
      activityBId: "transfer-in-id",
    });
    expect(mutationMocks.updateMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "transfer-in-id",
        accountId: "acc-cad",
        activityType: ActivityType.TRANSFER_IN,
        metadata: { flow: { is_external: true } },
      }),
    );
    expect(mutationMocks.unlinkMutateAsync.mock.invocationCallOrder[0]).toBeLessThan(
      mutationMocks.updateMutateAsync.mock.invocationCallOrder[0],
    );
  });
});
