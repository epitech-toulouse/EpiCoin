#!/usr/bin/env bash
# Backs up an EpiCoin platform run with docker compose.
#
#   scripts/backup.sh                  data: database, chain, genesis, product images
#   scripts/backup.sh --with-secrets   the same, plus the keys (validator, admin, treasury,
#                                      master key) in a separate archive
#
# The node is stopped for the few seconds of the backup, so that the database and the chain
# are captured at the same moment. Meanwhile the site stays up; payments and distributions
# wait and complete as soon as the node is back.
#
# Backups go to ./backups/<UTC date>/ (or $EPICOIN_BACKUP_DIR/<UTC date>/).
# - Data backups can be copied off-site regularly: they contain personal data, so store
#   them encrypted, but they are useless to an attacker without the keys.
# - The secrets archive gives full control of the platform. Make it once after the first
#   start and after every key rotation, and keep it OFFLINE (vault, encrypted USB key).
set -euo pipefail
cd "$(dirname "$0")/.."

with_secrets=false
case "${1:-}" in
  --with-secrets) with_secrets=true ;;
  "") ;;
  *) echo "usage: $0 [--with-secrets]" >&2; exit 2 ;;
esac

root="${EPICOIN_BACKUP_DIR:-./backups}"
mkdir -p "$root"
dest="$(cd "$root" && pwd)/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -m 700 "$dest"

compose() { docker compose "$@"; }
tools() { compose --profile tools run --rm --no-deps -T -v "$dest:/backup" tools "$@"; }

echo "Backing up to $dest"

# 1. Freeze the chain: from here on, nothing new can be included in a block.
compose stop node >/dev/null
restart_node() { compose start node >/dev/null; }
trap restart_node EXIT

# 2. The database. Every transaction on chain was recorded in it before being submitted,
#    so the dump knows about all of them (some may still be marked pending: they are
#    settled automatically after a restore).
compose exec -T postgres sh -c \
  'pg_dump --username="$POSTGRES_USER" --dbname="$POSTGRES_DB" --format=custom' \
  >"$dest/postgres.dump"

# 3. The chain, its genesis and the uploaded images.
tools sh -c 'tar -C / -czf /backup/chain.tar.gz data chain uploads'

restart_node
trap - EXIT

# 4. The keys, on request only.
if $with_secrets; then
  tools sh -c 'tar -C / -czf /backup/secrets.tar.gz secrets/node secrets/api'
fi

# Hand the files over to the user running the script, readable by them only.
tools sh -c "chown -R $(id -u):$(id -g) /backup && chmod 600 /backup/*"
(cd "$dest" && shasum -a 256 ./* >SHA256SUMS)

echo "Done:"
ls -l "$dest"
if $with_secrets; then
  echo "secrets.tar.gz holds every key of the platform: move it offline now."
fi
