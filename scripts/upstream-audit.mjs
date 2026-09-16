#!/usr/bin/env node
// Read-only upstream drift audit for this maintained Wealthfolio derivative.
//
// Contract (`docs/upstream-integration.md`):
// - resolves the pinned fork base (upstream.json), the comparison ref, and HEAD;
// - fails closed when a required ref is missing or `--check-remote` cannot
//   confirm the comparison ref still equals the live upstream branch;
// - runs read-only git commands only and writes exactly one artifact, the
//   reviewable proposal report;
// - never merges, rebases, resets, cherry-picks, force-pushes, or deploys.

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const EXIT = {
  ok: 0,
  usage: 2,
  missingRef: 3,
  upstreamUnresolved: 4,
  gitFailure: 5,
};

const SHA = /^[0-9a-f]{40}$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;
const RELEASE_TAG = /^v(\d+)\.(\d+)\.(\d+)$/;
const COMMIT_MARKER = "\u0001";

// Security-relevant upstream paths. Deliberately over-inclusive: a false
// positive costs one review, a false negative costs an unpatched credential
// path in this fork.
const SECURITY_PATH_GLOBS = [
  "security/**",
  ".github/workflows/**",
  "ci/**",
  "**/*auth*",
  "**/*credential*",
  "**/*crypto*",
  "**/*decrypt*",
  "**/*encrypt*",
  "**/*keychain*",
  "**/*keyring*",
  "**/*password*",
  "**/*permission*",
  "**/*secret*",
  "**/*secure*",
  "**/*token*",
  "apps/server/src/api/bridge.rs",
  "crates/storage-sqlite/src/bridge_policy.rs",
];

// Migration/storage-relevant upstream paths: they change the SQLite schema this
// fork ships, and therefore the databases the recovery producer exports.
const MIGRATION_PATH_GLOBS = [
  "**/*migration*",
  "crates/storage-sqlite/migrations/**",
  "crates/storage-sqlite/src/db/**",
  "crates/storage-sqlite/src/schema.rs",
];

// Bundled image/font assets inflate the raw file surface without adding
// integration risk, so the report counts them separately.
const ASSET_GLOBS = [
  "**/*.png",
  "**/*.svg",
  "**/*.webp",
  "**/*.jpg",
  "**/*.jpeg",
  "**/*.ico",
  "**/*.woff",
  "**/*.woff2",
  "**/*.ttf",
];

// Ambiguity in a finance codebase is real: "security" also means a tradable
// instrument and "expose" is ordinary vocabulary, so neither is a signal here.
// Matched against the subject only: upstream commit bodies carry templates,
// generated-by footers, and review checklists that produce nothing but noise.
const SECURITY_MESSAGE_PATTERNS = [
  /\b(authenticat|authoriz|unauthoriz|oauth)\w*|\bauth\b/i,
  /\bcve-\d{4}-\d+/i,
  /\b(sanitiz|traversal|injection|ssrf|xss|csrf|spoof|tamper)\w*/i,
  /\b(credential|secret|password|keychain|keyring|permission|encrypt|decrypt)\w*/i,
  /\b(bypass|exploit|vulnerab|unauthenticated|insecure|privilege)\w*/i,
  /\b(leak|hardening|advisory|disclosure)\w*/i,
  /\btokens?\b/i,
  /\bdenial of service\b/i,
];

// Ordered: the first rule that matches a path owns it.
const AREA_RULES = [
  { area: "storage", globs: ["crates/storage-sqlite/**"] },
  { area: "core", globs: ["crates/core/**"] },
  { area: "spending", globs: ["crates/spending/**"] },
  { area: "agent-tools", globs: ["crates/agent-tools/**"] },
  { area: "connect", globs: ["crates/connect/**"] },
  { area: "device-sync", globs: ["crates/device-sync/**"] },
  { area: "market-data", globs: ["crates/market-data/**"] },
  { area: "ai", globs: ["crates/ai/**"] },
  { area: "frontend", globs: ["apps/frontend/**"] },
  { area: "server", globs: ["apps/server/**"] },
  { area: "desktop", globs: ["apps/tauri/**"] },
  { area: "packages", globs: ["packages/**"] },
  { area: "crates-other", globs: ["crates/**"] },
  {
    area: "delivery",
    globs: [
      ".github/**",
      "ci/**",
      "deploy/**",
      "packaging/**",
      "e2e/**",
      "Dockerfile",
      "Dockerfile.dev",
      "compose*.yml",
      "*.sh",
    ],
  },
  {
    area: "dependencies",
    globs: [
      "Cargo.toml",
      "**/Cargo.toml",
      "Cargo.lock",
      "package.json",
      "**/package.json",
      "pnpm-lock.yaml",
      "pnpm-workspace.yaml",
      "renovate.json",
      "rust-toolchain.toml",
      "tsconfig*.json",
      "eslint*.js",
      "vite.config.*",
      "playwright.config.ts",
    ],
  },
  { area: "documentation", globs: ["docs/**", "**/*.md", "*.md"] },
];

const ACTION_LEGEND = [
  [
    "gate-migration-review",
    "the area changes storage or migration paths: review as a forward migration and check native-export receipt compatibility before backporting",
  ],
  [
    "prioritise-security-review",
    "the area changes credential, authentication, bridge, or CI-supply-chain paths: review and decide before routine work",
  ],
  [
    "defer-with-conflict-resolution",
    "routine files that overlap fork-modified files: expect conflicts against the fork invariants",
  ],
  ["defer-routine", "routine files with no fork overlap: integrate in the next reviewed cycle"],
];

function usage() {
  return [
    "Usage: node scripts/upstream-audit.mjs [options]",
    "",
    "Read-only upstream drift audit. Writes one proposal report and nothing else.",
    "",
    "Options:",
    "  --repo <path>        git checkout to audit (default: the enclosing repository)",
    "  --pin <path>         audit pin file (default: <repo>/upstream.json)",
    "  --report <path>      report path (default: <repo>/<pin audit.report>)",
    "  --date <YYYY-MM-DD>  report date; supply it to make the artifact reproducible",
    "  --check-remote       confirm the comparison ref equals the live upstream branch",
    "  --remote-url <url>   upstream URL for --check-remote (default: pin upstream.url)",
    "  --help               print this help",
    "",
    "Exit codes:",
    `  ${EXIT.ok}  audit completed`,
    `  ${EXIT.usage}  invalid arguments or invalid pin`,
    `  ${EXIT.missingRef}  a required ref (fork base or comparison ref) is missing`,
    `  ${EXIT.upstreamUnresolved}  the current upstream branch could not be resolved, or the comparison ref is stale`,
    `  ${EXIT.gitFailure}  a read-only git command failed`,
  ].join("\n");
}

function fail(code, message) {
  process.stderr.write(`upstream-audit: ${message}\n`);
  process.exit(code);
}

function sh(repo, args) {
  try {
    const stdout = execFileSync("git", ["-C", repo, "-c", "core.quotepath=false", ...args], {
      encoding: "utf8",
      maxBuffer: 512 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { status: 0, stdout };
  } catch (error) {
    return {
      status: typeof error.status === "number" ? error.status : 1,
      stdout: typeof error.stdout === "string" ? error.stdout : "",
      stderr: typeof error.stderr === "string" ? error.stderr : String(error.message ?? error),
    };
  }
}

function git(repo, args) {
  const result = sh(repo, args);
  if (result.status !== 0) {
    fail(EXIT.gitFailure, `git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  }
  return result.stdout.replace(/\n+$/, "");
}

function resolveCommit(repo, revision) {
  const result = sh(repo, ["rev-parse", "--verify", "--quiet", `${revision}^{commit}`]);
  const value = result.stdout.trim();
  return result.status === 0 && SHA.test(value) ? value : null;
}

function isAncestor(repo, ancestor, descendant) {
  const result = sh(repo, ["merge-base", "--is-ancestor", ancestor, descendant]);
  if (result.status !== 0 && result.status !== 1) {
    fail(EXIT.gitFailure, `git merge-base failed: ${result.stderr.trim()}`);
  }
  return result.status === 0;
}

function lines(value) {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

function byString(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function globToRegExp(pattern) {
  let output = "^";
  for (let index = 0; index < pattern.length; index += 1) {
    const character = pattern[index];
    if (character === "*") {
      if (pattern[index + 1] === "*") {
        index += 1;
        if (pattern[index + 1] === "/") {
          index += 1;
          output += "(?:.*/)?";
        } else {
          output += ".*";
        }
      } else {
        output += "[^/]*";
      }
    } else if (character === "?") {
      output += "[^/]";
    } else {
      output += character.replace(/[|\\{}()[\]^$+?.]/g, "\\$&");
    }
  }
  return new RegExp(`${output}$`);
}

const GLOB_CACHE = new Map();

function matchesGlob(globs, value) {
  return globs.some((glob) => {
    let pattern = GLOB_CACHE.get(glob);
    if (!pattern) {
      pattern = globToRegExp(glob);
      GLOB_CACHE.set(glob, pattern);
    }
    return pattern.test(value);
  });
}

function areaOf(file) {
  for (const rule of AREA_RULES) {
    if (matchesGlob(rule.globs, file)) return rule.area;
  }
  return "other";
}

function parseOptions(argv) {
  const options = {
    repo: null,
    pin: null,
    report: null,
    date: null,
    checkRemote: false,
    remoteUrl: null,
  };
  const take = (index, flag) => {
    const value = argv[index];
    if (value === undefined || value.startsWith("--")) {
      fail(EXIT.usage, `${flag} requires a value`);
    }
    return value;
  };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    switch (flag) {
      case "--repo":
        options.repo = take((index += 1), flag);
        break;
      case "--pin":
        options.pin = take((index += 1), flag);
        break;
      case "--report":
        options.report = take((index += 1), flag);
        break;
      case "--date":
        options.date = take((index += 1), flag);
        break;
      case "--remote-url":
        options.remoteUrl = take((index += 1), flag);
        break;
      case "--check-remote":
        options.checkRemote = true;
        break;
      case "--":
        // Package-manager argument separator (`pnpm run audit:upstream -- --date ...`).
        break;
      case "--help":
      case "-h":
        process.stdout.write(`${usage()}\n`);
        process.exit(EXIT.ok);
        break;
      default:
        fail(EXIT.usage, `unknown argument: ${flag}`);
    }
  }
  return options;
}

function loadPin(pinPath) {
  let document;
  try {
    document = JSON.parse(readFileSync(pinPath, "utf8"));
  } catch (error) {
    fail(EXIT.usage, `cannot read audit pin ${pinPath}: ${error.message}`);
  }
  const upstream = document?.upstream;
  const forkBase = document?.fork_base;
  const audit = document?.audit;
  const integration = document?.integration;
  const relative = (value) =>
    typeof value === "string" &&
    value.length > 0 &&
    !value.startsWith("/") &&
    !value.split("/").includes("..");
  if (
    document?.version !== 1 ||
    typeof upstream?.url !== "string" ||
    !/^https:\/\//.test(upstream.url) ||
    typeof upstream?.comparison_ref !== "string" ||
    !upstream.comparison_ref.startsWith("refs/") ||
    typeof upstream?.release_ref_pattern !== "string" ||
    !upstream.release_ref_pattern.startsWith("refs/tags/") ||
    typeof forkBase?.tag !== "string" ||
    !RELEASE_TAG.test(forkBase.tag) ||
    typeof forkBase?.commit !== "string" ||
    !SHA.test(forkBase.commit) ||
    typeof audit?.cadence !== "string" ||
    !["daily", "weekly", "monthly"].includes(audit.cadence) ||
    typeof audit?.workflow !== "string" ||
    !relative(audit.workflow) ||
    typeof audit?.report !== "string" ||
    !relative(audit.report) ||
    integration?.mode !== "reviewed-proposal" ||
    typeof integration?.decision_owner !== "string" ||
    integration.decision_owner.length === 0 ||
    typeof integration?.rule !== "string" ||
    integration.rule.length === 0
  ) {
    fail(EXIT.usage, `audit pin ${pinPath} does not satisfy the audit pin contract`);
  }
  return { upstream, forkBase, audit, integration };
}

// One read-only call yields each non-merge commit with its message and its
// complete changed-file list, so commit and file classification cannot diverge.
function readChanges(repo, range) {
  const raw = git(repo, [
    "log",
    "-z",
    "--no-renames",
    "--no-merges",
    "--name-only",
    "--format=\u0001%H%x1f%s",
    range,
  ]);
  const commits = new Map();
  let current = null;
  for (const token of raw.split("\0")) {
    if (!token) continue;
    if (token.startsWith(COMMIT_MARKER)) {
      const [sha, subject] = token.slice(1).split("\x1f");
      if (!SHA.test(sha) || subject === undefined) {
        fail(EXIT.gitFailure, `cannot parse commit metadata at ${sha ?? "unknown"}`);
      }
      current = { sha, subject: subject.trim(), files: [] };
      commits.set(sha, current);
      continue;
    }
    if (current === null) {
      fail(EXIT.gitFailure, `cannot parse changed files at ${token.trim()}`);
    }
    current.files.push(token.replace(/^\n+/, ""));
  }
  return commits;
}

function semver(tag) {
  const match = RELEASE_TAG.exec(tag);
  return match ? match.slice(1).map(Number) : null;
}

function releasesSinceBase(repo, pin, comparison, base) {
  const refs = lines(
    git(repo, [
      "for-each-ref",
      "--format=%(refname)%09%(objecttype)%09%(objectname)%09%(*objectname)",
      pin.upstream.release_ref_pattern,
    ]),
  ).map((line) => {
    const [ref, objecttype, objectname, peeled] = line.split("\t");
    return { ref, objecttype, objectname, peeled };
  });
  const releases = [];
  for (const ref of refs) {
    const tag = ref.ref.replace("refs/tags/", "");
    if (!semver(tag)) continue;
    const commit = ref.objecttype === "tag" && ref.peeled ? ref.peeled : ref.objectname;
    if (!SHA.test(commit)) continue;
    if (!isAncestor(repo, commit, comparison)) continue;
    if (isAncestor(repo, commit, base)) continue;
    releases.push({ tag, commit });
  }
  releases.sort((left, right) => {
    const leftVersion = semver(left.tag);
    const rightVersion = semver(right.tag);
    for (let index = 0; index < 3; index += 1) {
      if (leftVersion[index] !== rightVersion[index])
        return leftVersion[index] - rightVersion[index];
    }
    return byString(left.tag, right.tag);
  });
  return { available: refs.length > 0, releases };
}

function recommendedAction(area) {
  if (area.migrationFiles > 0) return "gate-migration-review";
  if (area.securityFiles > 0) return "prioritise-security-review";
  if (area.forkOverlapFiles > 0) return "defer-with-conflict-resolution";
  return "defer-routine";
}

function escapeCell(value) {
  return String(value).replace(/\|/g, "\\|");
}

function renderTable(headers, rows) {
  const header = `| ${headers.join(" | ")} |`;
  const divider = `| ${headers.map(() => "---").join(" | ")} |`;
  const body = rows.map((row) => `| ${row.map(escapeCell).join(" | ")} |`);
  return [header, divider, ...body].join("\n");
}

function renderReport(input) {
  const {
    date,
    pin,
    base,
    comparison,
    head,
    headRef,
    releases,
    remote,
    drift,
    buckets,
    areas,
    files,
  } = input;
  const commitRows = (commits) =>
    commits.map((commit) => [
      `\`${commit.sha.slice(0, 12)}\``,
      commit.subject,
      commit.flags.join(", "),
    ]);
  const areaRows = areas.map((area) => [
    area.area,
    String(area.files),
    String(area.securityFiles),
    String(area.migrationFiles),
    String(area.forkOverlapFiles),
    String(area.commits),
    area.recommendedAction,
  ]);
  const actionRows = ACTION_LEGEND.map(([action, description]) => [
    action,
    areas
      .filter((area) => area.recommendedAction === action)
      .map((area) => area.area)
      .join(", ") || "_none_",
    description,
  ]);
  const overlapRows = areas
    .filter((area) => area.overlapPaths.length > 0)
    .map((area) => [area.area, String(area.forkOverlapFiles), area.overlapPaths.join(", ")]);
  const releaseLines = releases.available
    ? releases.releases.length
      ? releases.releases.map((release) => `- \`${release.tag}\` ${release.commit}`)
      : ["- _none: no upstream release ref is ahead of the pinned fork base._"]
    : [
        "- _unavailable: no upstream release ref exists in this checkout._",
        "- Release-tag evidence is missing, so this audit cannot rank or confirm upstream releases.",
        `- Fetch the release refs before deciding: \`git fetch --tags ${pin.upstream.url}\`.`,
      ];
  const remoteLines = {
    confirmed: [
      `- Confirmed: \`${pin.upstream.url}\` \`refs/heads/main\` is \`${remote.commit}\`.`,
    ],
    "not-requested": [
      "- Not requested. Re-run with `--check-remote` to prove the comparison ref still equals the live upstream branch.",
    ],
  }[remote.state];

  return `${[
    "<!-- upstream-audit",
    JSON.stringify({
      audit_version: 1,
      date,
      fork_base: { tag: pin.forkBase.tag, commit: base },
      comparison_ref: pin.upstream.comparison_ref,
      comparison_commit: comparison,
      head_ref: headRef,
      head_commit: head,
      remote_check: remote.state,
      remote_main_commit: remote.commit,
      release_evidence: releases.available ? "available" : "unavailable",
      releases_since_base: releases.releases,
      commits_behind: drift.commitsBehind,
      commits_upstream_of_base: drift.upstreamOfBase,
      commits_upstream_of_base_change: drift.upstreamOfBaseChange,
      fork_only_commits: drift.forkOnly,
      changed_files: files.upstreamFiles.length,
      asset_files: files.assetFiles.length,
      fork_changed_files: files.forkFiles.length,
      fork_overlap_files: files.forkOverlapFiles.length,
      security_commits: buckets.security,
      migration_commits: buckets.migration,
      routine_commit_count: buckets.routine.length,
      areas: areas.map((area) => ({
        area: area.area,
        files: area.files,
        security_files: area.securityFiles,
        migration_files: area.migrationFiles,
        fork_overlap_files: area.forkOverlapFiles,
        commits: area.commits,
        recommended_action: area.recommendedAction,
      })),
      decision_owner: pin.integration.decision_owner,
      integration_mode: pin.integration.mode,
    }),
    "-->",
    "",
    `# Upstream audit proposal — ${date}`,
    "",
    "## Decision gate",
    "",
    "This artifact is a proposal. The audit performed no merge, rebase, reset,",
    "cherry-pick, force-push, or deployment, and no automation performs one.",
    "Upstream integration is a reviewed selective-backport pull request decided",
    `by the repository owner (\`${pin.integration.decision_owner}\`), per the integration`,
    "rule pinned in `upstream.json`. Resolve every conflict against the fork",
    "invariants listed in `plan.md` §9.2.",
    "",
    "## Provenance and inputs",
    "",
    renderTable(
      ["item", "value"],
      [
        ["audit pin", `\`upstream.json\` (pinned fork base \`${pin.forkBase.tag}\`)`],
        ["upstream repository", pin.upstream.url],
        ["pinned fork base", `\`${base}\``],
        ["comparison ref", `\`${pin.upstream.comparison_ref}\` = \`${comparison}\``],
        ["fork HEAD", `\`${head}\` (\`${headRef}\`)`],
        ["audit cadence", `${pin.audit.cadence} via \`${pin.audit.workflow}\``],
        ["decision owner", pin.integration.decision_owner],
      ],
    ),
    "",
    "## Drift",
    "",
    `- Commits behind upstream: **${drift.commitsBehind}** (\`HEAD..${pin.upstream.comparison_ref}\`).`,
    `- Upstream commits above the pinned base: **${drift.upstreamOfBase}** (\`${pin.forkBase.tag}..${pin.upstream.comparison_ref}\`), of which **${drift.upstreamOfBaseChange}** are non-merge changes.`,
    `- Fork-only commits: **${drift.forkOnly}** (\`${pin.upstream.comparison_ref}..HEAD\`).`,
    `- Changed-file surface between base and comparison ref: **${files.upstreamFiles.length}** files, of which **${files.assetFiles.length}** are bundled image or font assets.`,
    "",
    "## Release evidence",
    "",
    ...releaseLines,
    "",
    "## Remote confirmation",
    "",
    ...remoteLines,
    "",
    `## Security-relevant upstream changes (${buckets.security.length})`,
    "",
    buckets.security.length
      ? renderTable(["commit", "subject", "matched by"], commitRows(buckets.security))
      : "_None in this delta._",
    "",
    `## Migration-relevant upstream changes (${buckets.migration.length})`,
    "",
    buckets.migration.length
      ? renderTable(["commit", "subject", "matched by"], commitRows(buckets.migration))
      : "_None in this delta._",
    "",
    "## Routine upstream changes",
    "",
    `**${buckets.routine.length}** of **${drift.upstreamOfBaseChange}** non-merge commits are neither security- nor migration-relevant.`,
    `Enumerate them with \`git log --oneline --no-merges ${base}..${comparison}\` and subtract the commits listed above.`,
    "",
    "## Changed-file surface by area",
    "",
    "Columns: `files` are upstream-changed files owned by the area; `security`",
    "and `migration` count the area's files on those two path sets; `fork-overlap",
    "files` are files the fork also changed; `commits` are upstream non-merge",
    "commits touching the area; `recommended action` follows the legend below.",
    "",
    areaRows.length
      ? renderTable(
          [
            "area",
            "files",
            "security",
            "migration",
            "fork-overlap files",
            "commits",
            "recommended action",
          ],
          areaRows,
        )
      : "_No changed files in this delta._",
    "",
    "## Recommended action per area",
    "",
    renderTable(["action", "areas", "meaning"], actionRows),
    "",
    "## Fork overlap detail",
    "",
    overlapRows.length
      ? renderTable(["area", "overlapping files", "paths"], overlapRows)
      : "_No upstream-changed file is also changed by this fork._",
    "",
    "## Reproduction",
    "",
    "```bash",
    `git fetch --no-tags ${pin.upstream.url} '+refs/heads/main:${pin.upstream.comparison_ref}'`,
    `git fetch ${pin.upstream.url} '${pin.upstream.release_ref_pattern}:${pin.upstream.release_ref_pattern}'`,
    `node scripts/upstream-audit.mjs --date ${date} --check-remote`,
    "```",
    "",
    "## Non-mutation statement",
    "",
    "The audit ran read-only git commands (`rev-parse`, `rev-list`, `merge-base`,",
    "`diff`, `log`, `for-each-ref`, and `ls-remote` when `--check-remote` is set)",
    "and wrote exactly one file: this report. No ref, index, worktree, or object",
    "was created or modified, and no remote was contacted for anything other than",
    "a read.",
    "",
  ].join("\n")}`;
}

function main() {
  const options = parseOptions(process.argv.slice(2));
  const repo = options.repo
    ? path.resolve(options.repo)
    : git(process.cwd(), ["rev-parse", "--show-toplevel"]);
  const pinPath = options.pin ? path.resolve(options.pin) : path.join(repo, "upstream.json");
  const pin = loadPin(pinPath);
  const reportPath = options.report
    ? path.resolve(options.report)
    : path.join(repo, pin.audit.report);
  if (reportPath.includes(`${path.sep}.git${path.sep}`)) {
    fail(EXIT.usage, `--report must not write inside the git directory: ${reportPath}`);
  }
  const date = options.date ?? new Date().toISOString().slice(0, 10);
  if (!DATE.test(date)) fail(EXIT.usage, `--date must be YYYY-MM-DD, received ${date}`);

  const base = resolveCommit(repo, pin.forkBase.commit);
  if (!base) {
    fail(
      EXIT.missingRef,
      `pinned fork base ${pin.forkBase.tag} ${pin.forkBase.commit} is not present in ${repo}; fetch the pinned base before auditing`,
    );
  }
  if (!isAncestor(repo, base, "HEAD")) {
    fail(
      EXIT.missingRef,
      `pinned fork base ${base} is not an ancestor of HEAD; the pin is wrong or the history was rewritten`,
    );
  }
  const comparison = resolveCommit(repo, pin.upstream.comparison_ref);
  if (!comparison) {
    fail(
      EXIT.missingRef,
      `comparison ref ${pin.upstream.comparison_ref} is missing in ${repo}; fetch ${pin.upstream.url} before auditing`,
    );
  }
  if (!isAncestor(repo, base, comparison)) {
    fail(
      EXIT.missingRef,
      `pinned fork base ${base} is not an ancestor of ${pin.upstream.comparison_ref}; the pin is wrong`,
    );
  }
  const head = resolveCommit(repo, "HEAD");
  const headRef = git(repo, ["rev-parse", "--abbrev-ref", "HEAD"]);

  const remote = { state: "not-requested", commit: null };
  if (options.checkRemote) {
    const url = options.remoteUrl ?? pin.upstream.url;
    const result = sh(repo, ["ls-remote", url, "refs/heads/main"]);
    const live = result.stdout.split(/\s/)[0]?.trim() ?? "";
    if (result.status !== 0 || !SHA.test(live)) {
      fail(
        EXIT.upstreamUnresolved,
        `cannot resolve the current upstream branch from ${url}: ${result.stderr.trim() || "no refs/heads/main"}`,
      );
    }
    if (live !== comparison) {
      fail(
        EXIT.upstreamUnresolved,
        `${pin.upstream.comparison_ref} is stale: ${comparison} does not equal the live ${url} refs/heads/main ${live}; fetch it and rerun`,
      );
    }
    remote.state = "confirmed";
    remote.commit = live;
  }

  const range = `${base}..${comparison}`;
  const changes = readChanges(repo, range);
  const buckets = { security: [], migration: [], routine: [] };
  for (const commit of [...changes.values()].sort((left, right) => byString(left.sha, right.sha))) {
    const securityPath = commit.files.some((file) => matchesGlob(SECURITY_PATH_GLOBS, file));
    const migrationPath = commit.files.some((file) => matchesGlob(MIGRATION_PATH_GLOBS, file));
    const securitySubject = SECURITY_MESSAGE_PATTERNS.some((pattern) =>
      pattern.test(commit.subject),
    );
    const security = securityPath || securitySubject;
    const classified = {
      sha: commit.sha,
      subject: commit.subject,
      flags: [
        ...(securityPath ? ["security-path"] : []),
        ...(securitySubject ? ["security-subject"] : []),
        ...(migrationPath ? ["migration-path"] : []),
      ],
    };
    if (security) buckets.security.push(classified);
    else if (migrationPath) buckets.migration.push(classified);
    else buckets.routine.push(classified);
  }

  const upstreamFiles = lines(
    git(repo, ["diff", "--no-renames", "--name-only", base, comparison]),
  ).sort(byString);
  const forkFiles = lines(git(repo, ["diff", "--no-renames", "--name-only", base, "HEAD"])).sort(
    byString,
  );
  const forkFileSet = new Set(forkFiles);
  const assetFiles = upstreamFiles.filter((file) => matchesGlob(ASSET_GLOBS, file));
  const securityShas = new Set(buckets.security.map((commit) => commit.sha));
  const areaNames = [...new Set(upstreamFiles.map(areaOf))].sort(byString);
  const areas = areaNames
    .map((area) => {
      const areaFiles = upstreamFiles.filter((file) => areaOf(file) === area);
      const areaCommits = [...changes.values()].filter((commit) =>
        commit.files.some((file) => areaOf(file) === area),
      );
      const overlapPaths = areaFiles.filter((file) => forkFileSet.has(file));
      return {
        area,
        files: areaFiles.length,
        securityFiles: areaFiles.filter((file) => matchesGlob(SECURITY_PATH_GLOBS, file)).length,
        migrationFiles: areaFiles.filter((file) => matchesGlob(MIGRATION_PATH_GLOBS, file)).length,
        commits: areaCommits.length,
        securityCommits: areaCommits.filter((commit) => securityShas.has(commit.sha)).length,
        forkOverlapFiles: overlapPaths.length,
        overlapPaths,
      };
    })
    .map((area) => ({ ...area, recommendedAction: recommendedAction(area) }))
    .sort((left, right) => byString(left.area, right.area));

  const releases = releasesSinceBase(repo, pin, comparison, base);
  const report = renderReport({
    date,
    pin,
    base,
    comparison,
    head,
    headRef,
    releases,
    remote,
    drift: {
      commitsBehind: Number(git(repo, ["rev-list", "--count", `HEAD..${comparison}`])),
      upstreamOfBase: Number(git(repo, ["rev-list", "--count", range])),
      upstreamOfBaseChange: changes.size,
      forkOnly: Number(git(repo, ["rev-list", "--count", `${comparison}..HEAD`])),
    },
    buckets,
    areas,
    files: {
      upstreamFiles,
      assetFiles,
      forkFiles,
      forkOverlapFiles: upstreamFiles.filter((file) => forkFileSet.has(file)),
    },
  });

  mkdirSync(path.dirname(reportPath), { recursive: true });
  writeFileSync(reportPath, report, "utf8");
  process.stdout.write(`${reportPath}\n`);
  if (!releases.available) {
    process.stderr.write(
      `upstream-audit: warning: no release refs matched ${pin.upstream.release_ref_pattern}; the proposal reports release evidence as unavailable\n`,
    );
  }
  process.exit(EXIT.ok);
}

main();
