# Upstream integration

This is the provenance record for the maintained derivative of upstream
Wealthfolio. It states where the fork base is pinned, how drift is audited, and
who decides an upstream integration.

## Provenance

| Object               | Authority                                                                               |
| -------------------- | --------------------------------------------------------------------------------------- |
| Pinned fork base     | `upstream.json` (`fork_base.tag`, `fork_base.commit`), reviewed through a normal commit |
| Comparison ref       | `refs/remotes/upstream/main` in the audit checkout, fetched read-only at audit time     |
| Audit cadence        | weekly, `Sat 06:41 UTC`, plus manual `workflow_dispatch`                                |
| Audit workflow       | `.github/workflows/upstream-audit.yml`                                                  |
| Audit logic          | `scripts/upstream-audit.mjs`, behavioural tests in `scripts/upstream-audit.test.mjs`    |
| Proposal artifact    | `upstream-audit/proposal.md`, published as the audit run's summary (never committed)    |
| Merge decision owner | the repository owner (`integration.decision_owner` in `upstream.json`)                  |

`upstream.json` is the only machine-readable pin. `plan.md` §3.1 records the
same authority: version proposals are proposal-only and never a deployment, and
`plan.md` §9.2 defines how an approved integration is executed.

## Standing rule

Upstream integration is a **reviewed proposal**, never an automatic update.

- The audit and its workflow never merge, rebase, reset, cherry-pick,
  force-push, or deploy, and hold no write credential
  (`permissions: contents: read`, `persist-credentials: false`).
- The comparison ref is produced by a read-only `git fetch` of
  `https://github.com/wealthfolio/wealthfolio.git` inside the ephemeral audit
  checkout. No remote is configured for push, and the audit never publishes a
  branch, tag, issue, or pull request.
- A human opens the selective-backport pull request described in `plan.md` §9.2
  and resolves every conflict against the fork invariants listed there.
- The fork is never labelled as an upstream release unless the complete release
  was integrated; record a fork release with `upstream-base=` plus a backport
  ledger instead.
- Dependency updates remain Renovate's authority; this audit does not propose
  dependency changes.

## What one audit run produces

`scripts/upstream-audit.mjs` resolves the pinned fork base, `HEAD`, and the
comparison ref, then writes one artifact, `upstream-audit/proposal.md`, whose
leading HTML comment carries the same facts as machine-readable JSON for that
run:

- drift: commits behind, upstream commits since the base, fork-only commits, and
  the changed-file surface;
- upstream releases since the base (with an explicit
  `release_evidence: unavailable` value when no release ref is present, never a
  silent zero);
- the upstream delta split into security-relevant, migration-relevant, and
  routine commits;
- the changed-file surface per area with a recommended action, the fork overlap
  that predicts conflicts, and the reproduction commands.

Classification is deterministic and path- and subject-based:

- **security-relevant**: a commit touching credential, authentication, bridge,
  encryption, permission, or CI-workflow paths, or whose subject names
  authentication, credentials, tokens, permissions, or a vulnerability. Matched
  against the subject only: upstream commit bodies carry templates and
  generated-by footers that make them useless as a signal.
- **migration-relevant**: a commit touching migrations, the storage schema, or
  the SQLite database layer, because those change the schema this fork ships and
  the database the Homelab recovery producer exports.
- **routine**: everything else.

The per-area recommended action is decided by the area's own files (migration
files, then security files, then fork overlap, then routine) and every report
prints the exact legend it used. The report is a proposal: a recommended action
is a review order, not an approval.

## Running and validating it

```bash
pnpm test:audit          # behavioural audit and recovery-interface tests

git fetch --no-tags https://github.com/wealthfolio/wealthfolio.git \
  '+refs/heads/main:refs/remotes/upstream/main'
git fetch https://github.com/wealthfolio/wealthfolio.git \
  'refs/tags/v*:refs/tags/v*'
pnpm audit:upstream --date "$(date -u +%F)" --check-remote
```

Exit codes: `0` audited, `2` invalid arguments or pin, `3` a required ref is
missing, `4` the live upstream branch could not be resolved or the comparison
ref is stale, `5` a read-only git command failed.

Fail-closed behaviour, each covered by a behavioural test:

- a missing pinned fork base, a missing comparison ref, or a pinned base that is
  not an ancestor of `HEAD` writes no proposal and exits non-zero;
- `--check-remote` (used by the scheduled workflow) compares the comparison ref
  with the live `refs/heads/main` and fails closed when it is stale, so the
  proposal can never describe a stale upstream;
- the audit writes exactly one file and is proven not to change refs, history,
  or the index;
- identical inputs (including `--date`) produce a byte-identical report.

## Backport ledger

The pinned fork base is unchanged (`v3.6.1`). Selective backports taken since:

| Fork commit | Upstream commit           | Change                                               |
| ----------- | ------------------------- | ---------------------------------------------------- |
| #146        | `3605a5f73`               | Incremental valuation refreshes the latest saved day |
| #146        | `6c2e3c573` (server hunk) | Portfolio job continues after a market sync error    |
| #146        | `8297ed5fb`               | Bulk activity creates validate per row account       |
| #146        | `ba1aa61b1`, `fc2b1e498`  | Tax on cash activities is booked to cash             |
| #146        | `a7f51dc26`               | Non-ASCII symbols in Yahoo suffix parsing            |

Reviewed on 2026-10-03 against `upstream/main` `0deea7f31` and not yet taken:
`9c3b05e47` (OIDC JWKS refresh), `213cf4a95` + `76f559ccb` (static caching and
CSP hash), `c703f71b1` + `e15c92cc4` (day-first import dates), `5586c483c` +
`f979f606e` + `688065ff3` (spending drawer totals), and `487d23ffb`,
`c88328d4b`, `0830d14a5` (manual FX visibility, negative cash category, legacy
metadata cleanup).

## Updating the pin

Advance `fork_base` only after a reviewed integration lands, in the same commit
series that records the backport ledger, and only to a commit that is an
ancestor of the fork's `main`. Moving the pin without integrating the delta
hides drift from the next audit.
