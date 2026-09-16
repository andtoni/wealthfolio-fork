// Behavioural tests for the read-only upstream audit.
//
// Each test builds real git fixtures in a temporary directory and inspects the
// audit's exit code, its single report artifact, and the repository state
// before and after the run. Nothing here asserts report prose.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPT = path.join(path.dirname(fileURLToPath(import.meta.url)), "upstream-audit.mjs");
const REPOSITORY = path.resolve(path.dirname(SCRIPT), "..");
const PIN = path.join(REPOSITORY, "upstream.json");
const DATE = "2026-01-02";
const GIT_ENV = {
  ...process.env,
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_CONFIG_SYSTEM: "/dev/null",
  GIT_TERMINAL_PROMPT: "0",
  GIT_AUTHOR_DATE: "2026-01-01T00:00:00+0000",
  GIT_COMMITTER_DATE: "2026-01-01T00:00:00+0000",
};

function git(cwd, args) {
  return spawnSync("git", ["-C", cwd, ...args], { encoding: "utf8", env: GIT_ENV });
}

function mustGit(cwd, args) {
  const result = git(cwd, args);
  assert.equal(result.status, 0, `git ${args.join(" ")} failed: ${result.stderr}`);
  return result.stdout.trim();
}

function writeFile(repo, relative, content) {
  const target = path.join(repo, relative);
  mkdirSync(path.dirname(target), { recursive: true });
  writeFileSync(target, content, "utf8");
}

function commit(repo, message) {
  mustGit(repo, ["add", "-A"]);
  mustGit(repo, [
    "-c",
    "user.name=Fixture",
    "-c",
    "user.email=fixture@example.test",
    "commit",
    "--quiet",
    "--message",
    message,
  ]);
  return mustGit(repo, ["rev-parse", "HEAD"]);
}

function initRepo(directory) {
  mkdirSync(directory, { recursive: true });
  mustGit(directory, ["init", "--quiet", "--initial-branch=main"]);
  return directory;
}

function audit(args) {
  return spawnSync(process.execPath, [SCRIPT, ...args], { encoding: "utf8", env: GIT_ENV });
}

function reportHeader(reportPath) {
  const raw = readFileSync(reportPath, "utf8");
  const match = /<!-- upstream-audit\n([^\n]+)\n-->/.exec(raw);
  assert.ok(match, "report must carry a machine-readable audit header");
  return JSON.parse(match[1]);
}

function writePin(repo, options = {}) {
  const document = {
    version: 1,
    upstream: {
      url: "https://example.invalid/upstream.git",
      comparison_ref: "refs/remotes/upstream/main",
      release_ref_pattern: "refs/tags/v*",
    },
    fork_base: { tag: "v3.6.1", commit: options.base },
    audit: {
      cadence: "weekly",
      workflow: ".github/workflows/upstream-audit.yml",
      report: "upstream-audit/proposal.md",
    },
    integration: {
      mode: "reviewed-proposal",
      decision_owner: "fixture-owner",
      rule: "Reviewed selective backport only.",
    },
  };
  if (options.version !== undefined) document.version = options.version;
  writeFileSync(path.join(repo, "upstream.json"), `${JSON.stringify(document, null, 2)}\n`, "utf8");
}

// Fixture topology:
//   upstream: v3.6.1 base -> security commit -> migration commit -> routine commit (tag v3.6.2)
//   fork:     v3.6.1 base -> fork commit touching the same routine file
function buildFixture() {
  const root = mkdtempSync(path.join(tmpdir(), "upstream-audit-"));
  const fork = initRepo(path.join(root, "fork"));
  const upstream = initRepo(path.join(root, "upstream"));

  writeFile(fork, "README.md", "# fork\n");
  writeFile(fork, "crates/storage-sqlite/src/lib.rs", "// base\n");
  const base = commit(fork, "chore: base");
  mustGit(fork, ["tag", "v3.6.1"]);

  mustGit(upstream, ["symbolic-ref", "HEAD", "refs/heads/scratch"]);
  mustGit(upstream, ["fetch", "--quiet", fork, "refs/heads/main:refs/heads/main"]);
  mustGit(upstream, ["checkout", "--quiet", "main"]);
  writeFile(upstream, "apps/server/src/auth.rs", "// token\n");
  const security = commit(upstream, "fix(auth): rotate the bridge token");
  writeFile(
    upstream,
    "crates/storage-sqlite/migrations/2026-01-01-000001_add_index/up.sql",
    "SELECT 1;\n",
  );
  const migration = commit(upstream, "feat(storage): add an activity index");
  const routineFile = "apps/frontend/src/lib/utils.ts";
  writeFile(upstream, routineFile, "export const tooltip = 1;\n");
  const routine = commit(upstream, "chore(ui): tune the tooltip");
  mustGit(upstream, ["tag", "v3.6.2"]);

  writeFile(fork, routineFile, "export const tooltip = 2;\n");
  const forkCommit = commit(fork, "fix(ui): our own tooltip change");

  mustGit(fork, ["fetch", "--quiet", upstream, "refs/heads/main:refs/remotes/upstream/main"]);
  mustGit(fork, ["fetch", "--quiet", "--tags", upstream]);
  writePin(fork, { base });

  return { root, fork, upstream, base, security, migration, routine, forkCommit };
}

test("fails closed when the comparison ref is missing", () => {
  const { root, fork, base } = buildFixture();
  try {
    mustGit(fork, ["update-ref", "-d", "refs/remotes/upstream/main"]);
    const report = path.join(fork, "upstream-audit/proposal.md");
    const result = audit(["--repo", fork, "--date", DATE]);
    assert.equal(result.status, 3);
    assert.match(result.stderr, /refs\/remotes\/upstream\/main/);
    assert.equal(existsSync(report), false, "a failed audit must not write a proposal");
    assert.equal(mustGit(fork, ["rev-parse", "HEAD"]), mustGit(fork, ["rev-parse", "main"]));
    assert.ok(base);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("fails closed when the pinned fork base commit is unknown", () => {
  const { root, fork } = buildFixture();
  try {
    writePin(fork, { base: "0".repeat(40) });
    const report = path.join(fork, "upstream-audit/proposal.md");
    const result = audit(["--repo", fork, "--date", DATE]);
    assert.equal(result.status, 3);
    assert.match(result.stderr, /pinned fork base/);
    assert.equal(existsSync(report), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects an invalid audit pin without writing a proposal", () => {
  const { root, fork, base } = buildFixture();
  try {
    writePin(fork, { base, version: 2 });
    const result = audit(["--repo", fork, "--date", DATE]);
    assert.equal(result.status, 2);
    assert.match(result.stderr, /pin contract/);
    assert.equal(existsSync(path.join(fork, "upstream-audit/proposal.md")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("refuses to write the proposal inside the git directory", () => {
  const { root, fork } = buildFixture();
  try {
    const result = audit([
      "--repo",
      fork,
      "--date",
      DATE,
      "--report",
      path.join(fork, ".git/proposal.md"),
    ]);
    assert.equal(result.status, 2);
    assert.equal(existsSync(path.join(fork, ".git/proposal.md")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("fails closed when the comparison ref is stale against the live upstream branch", () => {
  const { root, fork, upstream } = buildFixture();
  try {
    writeFile(upstream, "apps/frontend/src/lib/utils.ts", "export const tooltip = 3;\n");
    commit(upstream, "chore(ui): move upstream ahead");
    const report = path.join(fork, "upstream-audit/proposal.md");
    const result = audit([
      "--repo",
      fork,
      "--date",
      DATE,
      "--check-remote",
      "--remote-url",
      upstream,
    ]);
    assert.equal(result.status, 4);
    assert.match(result.stderr, /stale/);
    assert.equal(existsSync(report), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("fails closed when the live upstream branch cannot be resolved", () => {
  const { root, fork } = buildFixture();
  try {
    const result = audit([
      "--repo",
      fork,
      "--date",
      DATE,
      "--check-remote",
      "--remote-url",
      path.join(root, "absent"),
    ]);
    assert.equal(result.status, 4);
    assert.match(result.stderr, /cannot resolve the current upstream branch/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("confirms the comparison ref and reports the drift, buckets and areas", () => {
  const { root, fork, upstream, base, security, migration, routine } = buildFixture();
  try {
    const report = path.join(fork, "upstream-audit/proposal.md");
    const result = audit([
      "--repo",
      fork,
      "--date",
      DATE,
      "--check-remote",
      "--remote-url",
      upstream,
    ]);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout.trim(), report);
    const header = reportHeader(report);

    assert.equal(header.fork_base.commit, base);
    assert.equal(header.comparison_commit, routine);
    assert.equal(header.remote_check, "confirmed");
    assert.equal(header.remote_main_commit, routine);
    assert.equal(header.commits_behind, 3);
    assert.equal(header.commits_upstream_of_base, 3);
    assert.equal(header.fork_only_commits, 1);
    assert.deepEqual(header.releases_since_base, [{ tag: "v3.6.2", commit: routine }]);
    assert.equal(header.release_evidence, "available");

    assert.deepEqual(
      header.security_commits.map((entry) => entry.sha),
      [security],
    );
    assert.deepEqual(header.security_commits[0].flags, ["security-path", "security-subject"]);
    assert.deepEqual(
      header.migration_commits.map((entry) => entry.sha),
      [migration],
    );
    assert.equal(header.routine_commit_count, 1);

    const byArea = Object.fromEntries(header.areas.map((area) => [area.area, area]));
    assert.equal(byArea.storage.recommended_action, "gate-migration-review");
    assert.equal(byArea.storage.security_files, 0);
    assert.equal(byArea.server.recommended_action, "prioritise-security-review");
    assert.equal(byArea.server.security_files, 1);
    assert.equal(byArea.frontend.recommended_action, "defer-with-conflict-resolution");
    assert.equal(byArea.frontend.fork_overlap_files, 1);
    assert.equal(header.fork_overlap_files, 1);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("reports an empty but available release set when no release is ahead of the base", () => {
  const { root, fork } = buildFixture();
  try {
    mustGit(fork, ["tag", "-d", "v3.6.2"]);
    const report = path.join(fork, "upstream-audit/proposal.md");
    const result = audit(["--repo", fork, "--date", DATE]);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, "");
    const header = reportHeader(report);
    assert.equal(header.release_evidence, "available");
    assert.deepEqual(header.releases_since_base, []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("reports unavailable release evidence without failing or claiming zero releases", () => {
  const { root, fork } = buildFixture();
  try {
    mustGit(fork, ["tag", "-d", "v3.6.2"]);
    mustGit(fork, ["tag", "-d", "v3.6.1"]);
    const report = path.join(fork, "upstream-audit/proposal.md");
    const result = audit(["--repo", fork, "--date", DATE]);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stderr, /release evidence as unavailable/);
    const header = reportHeader(report);
    assert.equal(header.release_evidence, "unavailable");
    assert.deepEqual(header.releases_since_base, []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("does not modify refs, history, or the index", () => {
  const { root, fork } = buildFixture();
  try {
    const snapshot = () => ({
      head: mustGit(fork, ["rev-parse", "HEAD"]),
      refs: mustGit(fork, [
        "for-each-ref",
        "--format=%(refname) %(objectname)",
        "refs/heads",
        "refs/tags",
        "refs/remotes",
      ]),
      objects: mustGit(fork, ["rev-list", "--objects", "--all"]),
      tracked: mustGit(fork, ["status", "--porcelain", "--untracked-files=no"]),
    });
    const before = snapshot();
    const result = audit(["--repo", fork, "--date", DATE]);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(snapshot(), before);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("produces the identical report for identical inputs", () => {
  const { root, fork } = buildFixture();
  try {
    const first = path.join(fork, "upstream-audit/first.md");
    const second = path.join(fork, "upstream-audit/second.md");
    assert.equal(audit(["--repo", fork, "--date", DATE, "--report", first]).status, 0);
    assert.equal(audit(["--repo", fork, "--date", DATE, "--report", second]).status, 0);
    assert.equal(readFileSync(first, "utf8"), readFileSync(second, "utf8"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("the shipped pin satisfies the audit contract and pins a real commit", () => {
  const pin = JSON.parse(readFileSync(PIN, "utf8"));
  assert.equal(pin.version, 1);
  assert.equal(pin.upstream.comparison_ref, "refs/remotes/upstream/main");
  assert.equal(pin.audit.cadence, "weekly");
  assert.equal(pin.audit.report, "upstream-audit/proposal.md");
  assert.equal(pin.integration.mode, "reviewed-proposal");
  assert.ok(pin.integration.decision_owner.length > 0);
  assert.match(pin.fork_base.commit, /^[0-9a-f]{40}$/);
  const present = git(REPOSITORY, ["cat-file", "-e", `${pin.fork_base.commit}^{commit}`]);
  if (present.status === 0) {
    // Full checkout: the pin must resolve to a real commit.
    return;
  }
  // Shallow or truncated checkout (the CI default): an unresolvable pinned base
  // must fail closed with the documented diagnostic, never pass silently.
  const shallow = audit(["--repo", REPOSITORY, "--date", DATE]);
  assert.equal(shallow.status, 3, "an unresolvable pinned base must fail closed");
  assert.match(shallow.stderr, /is not present/);
});
