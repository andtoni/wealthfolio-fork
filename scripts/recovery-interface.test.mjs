// The Homelab candidate-bound native-export producer
// (`homelab/scripts/create-wealthfolio-native-export-receipt`) finds the running
// Wealthfolio container by Compose labels, reads the container's single "/data"
// mount, opens "<mount>/wealthfolio.db" through a read-only SQLite connection,
// and requires `PRAGMA integrity_check` to return ok before it records a
// recovery receipt. These tests keep that interface an explicit, reviewable
// contract of the repository that ships the image, instead of an assumption
// hidden in another repository.
//
// The version-identity half is asserted here too: the paired Homelab release
// manifest records applied schema versions as `wealthfolio:<migration directory>`
// strings, so the migration directory names are the version identities the
// recovery evidence depends on.

import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const REPOSITORY = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MIGRATIONS = path.join(REPOSITORY, "crates/storage-sqlite/migrations");
const MIGRATION_NAME = /^\d{4}-\d{2}-\d{2}-\d{6}_[a-z0-9_]+$/;

function read(relative) {
  return readFileSync(path.join(REPOSITORY, relative), "utf8");
}

function imageDatabasePath() {
  const match = /^ENV\s+WF_DB_PATH=(\S+)\s*$/m.exec(read("Dockerfile"));
  assert.ok(match, "Dockerfile must set the canonical WF_DB_PATH default");
  return match[1];
}

test("the canonical image publishes the database interface the recovery producer reads", () => {
  const databasePath = imageDatabasePath();
  const directory = path.posix.dirname(databasePath);
  const file = path.posix.basename(databasePath);

  // The producer joins the container's mount destination with "wealthfolio.db".
  assert.equal(file, "wealthfolio.db");
  assert.equal(directory, "/data");

  const volume = /^VOLUME\s+\[(.*)\]\s*$/m.exec(read("Dockerfile"));
  assert.ok(volume, "Dockerfile must declare the data volume");
  const declared = volume[1]
    .split(",")
    .map((entry) => entry.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
  assert.ok(
    declared.includes(directory),
    `the data volume for ${databasePath} must be declared, found ${JSON.stringify(declared)}`,
  );
});

test("the repository Compose definition keeps the same database file inside the data volume", () => {
  const compose = read("compose.yml");
  const databasePath = imageDatabasePath();
  const declared = [...compose.matchAll(/WF_DB_PATH:\s*"?([^"\s]+)"?/g)].map((match) => match[1]);
  assert.equal(declared.length, 1, "compose.yml must set WF_DB_PATH exactly once");
  assert.equal(declared[0], databasePath);

  const mounts = [...compose.matchAll(/-\s*([A-Za-z0-9_.-]+):(\/[A-Za-z0-9_.-]+)/g)].map(
    (match) => match[2],
  );
  assert.ok(
    mounts.includes(path.posix.dirname(databasePath)),
    `compose.yml must mount the database directory ${path.posix.dirname(databasePath)}`,
  );
});

test("migration directory names are the unique, ordered version identities", () => {
  const directories = readdirSync(MIGRATIONS, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name);

  assert.ok(directories.length > 0, "the fork must ship migrations");
  for (const name of directories) {
    assert.match(name, MIGRATION_NAME, `migration ${name} is not a version identity`);
    assert.ok(
      existsSync(path.join(MIGRATIONS, name, "up.sql")),
      `migration ${name} must carry up.sql`,
    );
  }
  assert.deepEqual(
    directories,
    [...directories].sort(),
    "migration identities must sort in application order",
  );
});
