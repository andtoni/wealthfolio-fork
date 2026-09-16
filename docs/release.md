# Release

Record build, package, provenance, publication, compatibility, and rollback
contracts when this workstream releases artifacts.

## Paired recovery interface

Homelab deploys this application through a controlled candidate whose recovery
evidence is produced by
`infrastructure/homelab/scripts/create-wealthfolio-native-export-receipt`. That
producer reads exactly one database path from the running container: the file
`wealthfolio.db` inside the container's single `/data` mount. It copies the file
through a read-only SQLite connection, requires `PRAGMA integrity_check` to
return `ok`, and only then records a candidate-bound receipt.

This repository owns that interface, so it is kept explicit and guarded by
`scripts/recovery-interface.test.mjs`:

- the canonical image sets `WF_DB_PATH=/data/wealthfolio.db` and declares the
  `/data` volume, so the producer's path is the image default rather than a
  deployment accident;
- `compose.yml` keeps the same value and mounts the same directory;
- storage stays a single SQLite file that a read-only connection can walk while
  the application runs;
- applied schema versions live in `__diesel_schema_migrations`, and a migration
  directory name is the version identity the paired release manifest records as
  `wealthfolio:<directory>`.

Not owned here: restoring the exported copy into an isolated exact-digest
container, and comparing the copy's applied migration version with the release
manifest, are producer responsibilities in Homelab. The current producer
integrity-checks the SQLite copy but does not perform either check, and the
manifest's `migration_versions` entries are not compared against the running
database by any admission step.
