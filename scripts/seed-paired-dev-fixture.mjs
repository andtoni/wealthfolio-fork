#!/usr/bin/env node

const base = (process.env.WF_URL ?? "https://wealthfolio-paired.dev.example.com").replace(
  /\/$/,
  "",
);
const password = process.env.WF_AUTH_PASSWORD;
if (!password) throw new Error("WF_AUTH_PASSWORD is required");
let cookie = "";

async function request(path, options = {}) {
  const response = await fetch(`${base}/api/v1${path}`, {
    ...options,
    headers: {
      "content-type": "application/json",
      ...(cookie ? { cookie } : {}),
      ...(options.headers ?? {}),
    },
  });
  const text = await response.text();
  let body;
  try {
    body = text ? JSON.parse(text) : undefined;
  } catch {
    body = text;
  }
  if (!response.ok)
    throw new Error(
      `${options.method ?? "GET"} ${path} ${response.status}: ${JSON.stringify(body)}`,
    );
  return { body, response };
}

async function waitForHealth() {
  for (let attempt = 1; attempt <= 90; attempt += 1) {
    try {
      await request("/healthz");
      return;
    } catch (error) {
      if (attempt === 90) throw error;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
  }
}

async function login() {
  const { response } = await request("/auth/login", {
    method: "POST",
    body: JSON.stringify({ password }),
  });
  cookie = response.headers.get("set-cookie")?.split(";", 1)[0] ?? "";
  if (!cookie) throw new Error("Wealthfolio login did not return a session cookie");
}

async function waitForInvestecFixture() {
  let lastError;
  for (let attempt = 1; attempt <= 60; attempt += 1) {
    try {
      const discovered = (await request("/provider-connections/investec/accounts")).body;
      if (!Array.isArray(discovered))
        throw new Error("Investec discovery response was not an array");
      const fixture = discovered.find(
        (account) => account.providerAccountId === "fixture-investec-card",
      );
      if (fixture) return fixture;
      lastError = new Error("Synthetic Investec fixture account is not available yet");
    } catch (error) {
      lastError = error;
    }
    if (attempt < 60) await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  throw lastError;
}

await waitForHealth();
await login();

let accounts = (await request("/accounts")).body;
if (!Array.isArray(accounts)) throw new Error("Accounts response was not an array");
let account = accounts.find((item) => item.providerAccountId === "fixture-investec-card");
const fixture = account ? undefined : await waitForInvestecFixture();

if (fixture) {
  const admitted = await request("/provider-connections/investec/admit", {
    method: "POST",
    body: JSON.stringify({
      connectionId: fixture.connectionId,
      providerAccountId: fixture.providerAccountId,
      name: fixture.name,
    }),
  });
  account = { ...(account ?? {}), id: admitted.body.accountId };
}

if (!account?.id)
  throw new Error("Synthetic Investec account admission did not return an account ID");

let activities = [];
for (let attempt = 1; attempt <= 30; attempt += 1) {
  const response = await request("/activities/search", {
    method: "POST",
    body: JSON.stringify({
      page: 0,
      pageSize: 100,
      accountIdFilter: [account.id],
      dateFrom: "2026-08-01",
      dateTo: "2026-08-31",
      sort: { id: "date", desc: false },
    }),
  });
  activities = Array.isArray(response.body?.data) ? response.body.data : [];
  const providerActivities = activities.filter((item) => item.sourceSystem === "FINANCE");
  const pending = providerActivities.filter((item) => item.status === "PENDING");
  const posted = providerActivities.filter((item) => item.status === "POSTED");
  const expected = [
    ["PENDING", 1357.38],
    ["PENDING", 1433.86],
    ["POSTED", 123.45],
    ["POSTED", 1345.13],
    ["POSTED", 1430.4],
    ["POSTED", 50],
  ];
  const hasExpectedMatrix =
    providerActivities.length === expected.length &&
    expected.every(([status, amount]) =>
      providerActivities.some(
        (item) =>
          item.status === status &&
          item.currency === "ZAR" &&
          Math.abs(Math.abs(Number(item.amount)) - amount) < 0.001,
      ),
    );
  if (hasExpectedMatrix && pending.length === 2 && posted.length === 4) {
    console.log(
      JSON.stringify(
        {
          accountId: account.id,
          message: "Synthetic paired fixture is ready.",
          pending: pending.map((item) => ({
            id: item.id,
            sourceRecordId: item.sourceRecordId,
            amount: item.amount,
            currency: item.currency,
            comment: item.comment,
          })),
          posted: posted.map((item) => ({
            id: item.id,
            sourceRecordId: item.sourceRecordId,
            amount: item.amount,
            currency: item.currency,
            comment: item.comment,
          })),
          next: "Open the activity menu on a pending SAMPLEMART ONLINE row and choose Link card authorization.",
        },
        null,
        2,
      ),
    );
    process.exit(0);
  }
  if (attempt < 30) await new Promise((resolve) => setTimeout(resolve, 2000));
}

throw new Error(
  `Synthetic fixture account admitted but activities did not converge: ${activities.length} activities found`,
);
