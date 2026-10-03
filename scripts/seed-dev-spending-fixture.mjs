#!/usr/bin/env node
// Fills the disposable paired dev stack with a few months of synthetic household
// spending so the Transactions and Budget screens can be judged on realistic data.
//
// Run inside the dev container (it needs the dev SQLite file):
//   docker compose -f compose.paired.dev.yml exec -e WF_AUTH_PASSWORD=... wealthfolio \
//     node scripts/seed-dev-spending-fixture.mjs
//
// Synthetic data only. Refuses to run outside the paired dev deployment.
import { DatabaseSync } from "node:sqlite";

const base = (process.env.WF_URL ?? "http://127.0.0.1:8088").replace(/\/$/, "");
const dbPath = process.env.WF_DB_PATH ?? "/data/wealthfolio.db";
const password = process.env.WF_AUTH_PASSWORD;
if (!password) throw new Error("WF_AUTH_PASSWORD is required");
if (process.env.WF_DEPLOYMENT_ID !== "wealthfolio-paired-dev")
  throw new Error("this seeder only runs in the wealthfolio-paired-dev deployment");

const MARKER = "Seed fixture marker";
const SPENDING = "spending_categories";
const INCOME = "income_sources";
let cookie = "";

async function request(path, options = {}) {
  const response = await fetch(`${base}/api/v1${path}`, {
    ...options,
    headers: { "content-type": "application/json", ...(cookie ? { cookie } : {}) },
  });
  const text = await response.text();
  if (!response.ok)
    throw new Error(`${options.method ?? "GET"} ${path} ${response.status}: ${text}`);
  return { body: text ? JSON.parse(text) : undefined, response };
}
const post = (path, body) => request(path, { method: "POST", body: JSON.stringify(body) });
const put = (path, body) => request(path, { method: "PUT", body: JSON.stringify(body) });

// Deterministic pseudo-random numbers so every seed produces the same fixture.
let state = 20261003;
function random() {
  state = (state * 1664525 + 1013904223) % 4294967296;
  return state / 4294967296;
}
const between = (low, high) => Math.round(low + random() * (high - low));
const pick = (items) => items[Math.floor(random() * items.length)];

const login = await post("/auth/login", { password });
cookie = login.response.headers.get("set-cookie")?.split(";", 1)[0] ?? "";
if (!cookie) throw new Error("login did not return a session cookie");

const accounts = (await request("/accounts")).body;
const account = accounts.find((item) => item.providerAccountId === "fixture-investec-card");
if (!account) throw new Error("run scripts/seed-paired-dev-fixture.mjs first");
await put("/settings", { baseCurrency: account.currency, timezone: "Africa/Johannesburg" });
await put("/spending/settings", { enabled: true, accountIds: [account.id] });

async function topLevel(taxonomyId) {
  const taxonomy = (await request(`/taxonomies/${taxonomyId}`)).body;
  return new Map(
    taxonomy.categories.filter((item) => !item.parentId).map((item) => [item.name, item.id]),
  );
}
const spending = await topLevel(SPENDING);
const income = await topLevel(INCOME);

// Budget groups: every category starts in the excluded "Other" group, which
// leaves the budget screen and the rings empty.
const groups = {
  Essentials: [
    "Housing",
    "Groceries",
    "Bills & Utilities",
    "Health & Wellness",
    "Transportation",
    "Education",
  ],
  Lifestyle: ["Food & Dining", "Shopping", "Entertainment", "Personal Care"],
  Giving: ["Gifts & Donations"],
};
let snapshot = (await request("/spending/budget")).body;
for (const [name, categories] of Object.entries(groups)) {
  if (!snapshot.state.groups.some((group) => group.name === name))
    snapshot = (await post("/spending/budget/groups", { name })).body;
  const group = snapshot.state.groups.find((item) => item.name === name);
  for (const category of categories) {
    const categoryId = spending.get(category);
    if (categoryId)
      await post("/spending/budget/group-assignments", {
        groupId: group.id,
        taxonomyId: SPENDING,
        categoryId,
      });
  }
}

const existing = (await post("/spending/cash-activities/search", { search: MARKER, limit: 1 }))
  .body;
if (existing.totalCount > 0) {
  console.log("spending fixture already seeded");
  process.exit(0);
}

// [category, merchants, [low, high] in ZAR, purchases per week]
const habits = [
  ["Groceries", ["Woolworths Food", "Checkers", "Pick n Pay", "Spar"], [180, 1900], 2.6],
  [
    "Food & Dining",
    ["Tashas", "Nando's", "Vida e Caffe", "Uber Eats", "Rocomamas"],
    [65, 780],
    1.9,
  ],
  ["Transportation", ["Engen", "Shell", "Uber", "BP"], [90, 1350], 1.2],
  ["Shopping", ["Takealot", "Mr Price", "Woolworths", "Yuppiechef"], [150, 2400], 0.6],
  ["Health & Wellness", ["Clicks", "Dis-Chem", "Virgin Active"], [120, 950], 0.5],
  ["Entertainment", ["Ster-Kinekor", "Computicket", "Exclusive Books"], [110, 720], 0.35],
  ["Personal Care", ["Sorbet", "Barber Co"], [180, 650], 0.2],
  ["Gifts & Donations", ["NetFlorist", "GivenGain"], [200, 1200], 0.12],
];
// [category, merchant, amount, day of month]
const fixed = [
  ["Housing", "Bond repayment", 15000, 1],
  ["Bills & Utilities", "City Power", 1500, 3],
  ["Bills & Utilities", "Vodacom", 900, 5],
  ["Health & Wellness", "Discovery Health", 4000, 2],
  ["Bills & Utilities", "Outsurance", 1500, 1],
  ["Entertainment", "Netflix", 199, 12],
  ["Entertainment", "Spotify", 79, 14],
  ["Education", "Course fees", 5000, 28],
];

const start = new Date(Date.UTC(2026, 5, 25));
const today = new Date();
const rows = [];
for (let day = new Date(start); day <= today; day.setUTCDate(day.getUTCDate() + 1)) {
  const date = day.toISOString().slice(0, 10);
  const stamp = (hour) => `${date}T${String(hour).padStart(2, "0")}:${between(10, 59)}:00+02:00`;
  if (day.getUTCDate() === 25)
    rows.push({ type: "DEPOSIT", notes: "Salary", amount: 50000, at: stamp(6), income: "Salary" });
  for (const [category, merchant, amount, dayOfMonth] of fixed)
    if (day.getUTCDate() === dayOfMonth)
      rows.push({ type: "WITHDRAWAL", notes: merchant, amount, at: stamp(7), category });
  for (const [category, merchants, [low, high], perWeek] of habits)
    if (random() < perWeek / 7)
      rows.push({
        type: "WITHDRAWAL",
        notes: pick(merchants),
        amount: between(low, high),
        at: stamp(between(8, 20)),
        // A few rows stay uncategorized so the review states have something to show.
        category: random() < 0.07 ? null : category,
      });
}
rows.push({
  type: "WITHDRAWAL",
  notes: MARKER,
  amount: 1,
  at: `${today.toISOString().slice(0, 10)}T06:00:00+02:00`,
  category: null,
});

const seeded = [];
for (const row of rows) {
  const activity = (
    await post("/activities", {
      accountId: account.id,
      activityType: row.type,
      activityDate: row.at,
      amount: String(row.amount),
      currency: account.currency,
      notes: row.notes,
    })
  ).body;
  seeded.push(activity.id);
  const categoryId = row.income
    ? (income.get(row.income) ?? [...income.values()][0])
    : row.category && spending.get(row.category);
  if (categoryId)
    await put(`/spending/activities/${encodeURIComponent(activity.id)}/assignments`, {
      taxonomyId: row.income ? INCOME : SPENDING,
      categoryId,
    });
}

// Monthly budgets, so the rings and the budget screen have targets to measure against.
const budgets = {
  Housing: 18500,
  Groceries: 9500,
  "Food & Dining": 3500,
  Transportation: 4200,
  Shopping: 3000,
  "Health & Wellness": 6200,
  "Bills & Utilities": 4600,
  Entertainment: 900,
  Education: 5200,
  "Personal Care": 700,
  "Gifts & Donations": 600,
};
for (const [name, amount] of Object.entries(budgets)) {
  const categoryId = spending.get(name);
  if (categoryId)
    await post("/spending/budget/targets", {
      periodKey: "default",
      targetType: "category",
      taxonomyId: SPENDING,
      categoryId,
      amount: String(amount),
    });
}

// Budget and Insights totals only count provider-verified rows. Manual rows have
// no provenance record, so mark the seeded ones the way the bridge marks its own.
const db = new DatabaseSync(dbPath);
db.exec("PRAGMA busy_timeout = 10000");
const insert = db.prepare(
  "INSERT OR IGNORE INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('activity',?,'live_canonical','finance',?,'dev-spending-fixture')",
);
for (const id of seeded) insert.run(id, `dev-spending-fixture:${id}`);
db.close();

console.log(`seeded ${seeded.length} transactions and ${Object.keys(budgets).length} budgets`);
