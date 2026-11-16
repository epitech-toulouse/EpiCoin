#!/usr/bin/env bash
# Restores an EpiCoin platform from a backup made by scripts/backup.sh.
#
#   scripts/restore.sh backups/20261116T120000Z
#
# Works on the same machine or on a new one (after `git clone`). Everything currently in
# the platform's volumes is REPLACED. The keys come from the backup's secrets.tar.gz if it
# has one; otherwise the keys already in place are kept (they must be the same chain's).
#
# Before the platform restarts, the whole chain is re-verified from its genesis.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ $# -ne 1 ] || [ ! -f "$1/postgres.dump" ] || [ ! -f "$1/chain.tar.gz" ]; then
  echo "usage: $0 <backup directory containing postgres.dump and chain.tar.gz>" >&2
  exit 2
fi
src="$(cd "$1" && pwd)"
(cd "$src" && shasum -a 256 -c SHA256SUMS >/dev/null) || {
  echo "checksum mismatch in $src: the backup is damaged" >&2
  exit 1
}

compose() { docker compose "$@"; }
tools() { compose --profile tools run --rm --no-deps -T -v "$src:/backup:ro" tools "$@"; }

echo "Stopping the platform…"
compose stop web api node >/dev/null 2>&1 || true
compose up -d --wait postgres >/dev/null

echo "Restoring the chain, its genesis and the images…"
tools sh -c 'rm -rf /data/* /chain/* /uploads/* && tar -C / -xzf /backup/chain.tar.gz'

if [ -f "$src/secrets.tar.gz" ]; then
  echo "Restoring the keys…"
  tools sh -c 'rm -rf /secrets/node/* /secrets/api/* && tar -C / -xzf /backup/secrets.tar.gz'
else
  tools sh -c 'test -s /secrets/node/keys.env && test -s /secrets/api/keys.env' || {
    echo "No keys in this backup and none in place: restore them first (secrets.tar.gz)." >&2
    exit 1
  }
fi
tools sh -c 'chown -R 10001:10001 /data /chain /uploads /secrets/node /secrets/api'

echo "Verifying the whole chain from its genesis…"
tools epicoin-cli verify --genesis /chain/genesis.json --data-dir /data

echo "Restoring the database…"
compose exec -T postgres sh -c \
  'dropdb --username="$POSTGRES_USER" --if-exists --force "$POSTGRES_DB" &&
   createdb --username="$POSTGRES_USER" "$POSTGRES_DB"'
compose exec -T postgres sh -c \
  'pg_restore --username="$POSTGRES_USER" --dbname="$POSTGRES_DB" --no-owner --exit-on-error' \
  <"$src/postgres.dump"

echo "Starting the platform…"
compose up -d --wait node api web >/dev/null
echo "Restored from $src."
