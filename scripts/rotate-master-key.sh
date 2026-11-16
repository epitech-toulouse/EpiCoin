#!/usr/bin/env bash
# Replaces the master key that encrypts the custodial wallets.
#
#   scripts/rotate-master-key.sh
#
# 1. backs up everything, keys included (under the current key);
# 2. stops the API and re-encrypts every wallet under a new random key (all or nothing);
# 3. stores the new key in place of the old one and restarts the API;
# 4. backs up the keys again.
# The wallets' keys themselves — hence addresses and balances — do not change.
#
# For keys kept in the api-secrets volume (the default). If EPICOIN_MASTER_KEY is set in
# .env instead, follow the manual procedure in docs/guide-admin.md.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -f .env ] && grep -Eq '^EPICOIN_MASTER_KEY=.+' .env; then
  echo "EPICOIN_MASTER_KEY is set in .env: see docs/guide-admin.md for the manual procedure." >&2
  exit 1
fi

compose() { docker compose "$@"; }
tools() { compose --profile tools run --rm --no-deps -T tools "$@"; }

old="$(tools sh -c 'sed -n "s/^EPICOIN_MASTER_KEY=//p" /secrets/api/keys.env')"
if [ -z "$old" ]; then
  echo "no master key found in the api-secrets volume" >&2
  exit 1
fi
new="$(compose run --rm --no-deps -T api epicoin-api generate-master-key | tr -d '[:space:]')"

echo "1/4 Backup with the current keys"
scripts/backup.sh --with-secrets >/dev/null

echo "2/4 Re-encrypting the wallets"
compose stop api >/dev/null
trap 'compose start api >/dev/null' EXIT
# Keys travel as environment variables, never as command-line arguments.
EPICOIN_MASTER_KEY="$new" EPICOIN_MASTER_KEY_PREVIOUS="$old" \
  compose run --rm --no-deps -T api epicoin-api rotate-master-key

echo "3/4 Installing the new key"
NEW_MASTER_KEY="$new" compose --profile tools run --rm --no-deps -T -e NEW_MASTER_KEY tools \
  sh -c 'sed -i "s/^EPICOIN_MASTER_KEY=.*/EPICOIN_MASTER_KEY=$NEW_MASTER_KEY/" /secrets/api/keys.env'
trap - EXIT
compose up -d --wait api >/dev/null

echo "4/4 Backup of the new keys"
scripts/backup.sh --with-secrets | tail -n 2
echo "Master key replaced. Destroy the secrets archives that hold the old key."
