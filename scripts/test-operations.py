#!/usr/bin/env python3
"""End-to-end test of the operating procedures: backup, restore, master-key rotation.

1. Starts a fresh platform (its own compose project and ports), with demo data.
2. Uses it: a student buys a product.
3. Backs it up with scripts/backup.sh --with-secrets.
4. Changes it after the backup, then DESTROYS every volume: a blank machine.
5. Restores with scripts/restore.sh.
6. Checks that the platform is exactly as it was at backup time, and still works: the
   restored keys sign new payments, the chain verifies.
7. Rotates the master key and checks that every wallet still signs.

Usage: scripts/test-operations.py   (needs Docker; takes a few minutes)
Only Python's standard library is used.
"""

import http.cookiejar
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PROJECT = "epicoin-restoretest"
WEB_PORT = 8097
BASE = f"http://127.0.0.1:{WEB_PORT}"
ENV = {
    **os.environ,
    "COMPOSE_PROJECT_NAME": PROJECT,
    "EPICOIN_WEB_PORT": str(WEB_PORT),
    "EPICOIN_POSTGRES_PORT": "5437",
    "EPICOIN_MAILPIT_PORT": "8037",
    "EPICOIN_BLOCK_INTERVAL_MS": "500",
}
PASSWORD = "epicoin-demo"
STUDENT = "camille.martin@epitech.eu"
ADMIN = "admin.demo@epitech.eu"


def run(*command, **kwargs):
    print("$", " ".join(command), flush=True)
    return subprocess.run(command, cwd=REPO, env=ENV, check=True, **kwargs)


def compose(*args, **kwargs):
    return run("docker", "compose", *args, **kwargs)


class Client:
    """A browser-like HTTP client: cookie jar and CSRF token."""

    def __init__(self):
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
        )
        self.csrf = None

    def call(self, method, path, body=None, headers=None):
        request = urllib.request.Request(
            BASE + path,
            data=None if body is None else json.dumps(body).encode(),
            method=method,
        )
        request.add_header("Content-Type", "application/json")
        if self.csrf and method != "GET":
            request.add_header("X-CSRF-Token", self.csrf)
        for name, value in (headers or {}).items():
            request.add_header(name, value)
        try:
            with self.opener.open(request) as response:
                status, text = response.status, response.read().decode()
        except urllib.error.HTTPError as error:
            status, text = error.code, error.read().decode()
        data = json.loads(text) if text else None
        if isinstance(data, dict) and "csrf_token" in data:
            self.csrf = data["csrf_token"]
        return status, data

    def login(self, email):
        status, _ = self.call("POST", "/api/auth/login", {"email": email, "password": PASSWORD})
        assert status == 200, f"login of {email} failed: {status}"
        return self

    def buy(self, product_id):
        return self.call(
            "POST",
            "/api/shop/orders",
            {"items": [{"product_id": product_id, "quantity": 1}]},
            {"Idempotency-Key": str(uuid.uuid4())},
        )


def wait_until_up(timeout=600):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(BASE + "/api/health", timeout=3) as response:
                if json.load(response).get("status") == "ok":
                    return
        except (OSError, ValueError):
            pass
        time.sleep(2)
    raise SystemExit("the platform did not come up")


def snapshot():
    """Everything that must survive a backup and restore unchanged."""
    admin = Client().login(ADMIN)
    student = Client().login(STUDENT)
    _, chain = admin.call("GET", "/api/explorer/status")
    _, stats = admin.call("GET", "/api/admin/stats")
    _, users = admin.call("GET", "/api/admin/users?limit=200")
    _, wallet = student.call("GET", "/api/me/wallet")
    _, history = student.call("GET", "/api/me/transactions?limit=200")
    _, orders = student.call("GET", "/api/shop/orders")
    _, products = student.call("GET", "/api/shop/products")
    _, audit = admin.call("GET", "/api/admin/audit?limit=200")
    return {
        "height": chain["height"],
        "state_root": chain["state_root"],
        "tip_hash": chain["tip_hash"],
        "stats": stats,
        "users": sorted(u["email"] for u in users["items"]),
        "wallet": wallet,
        "history": history,
        "orders": orders,
        "stock": sorted((p["name"], p["stock"]) for p in products),
        # Logins are audited too: compare what existed at backup time.
        "audit_before": None if audit is None else [e for e in audit["items"]],
    }


def main():
    backups = Path(tempfile.mkdtemp(prefix="epicoin-backups-"))
    ENV["EPICOIN_BACKUP_DIR"] = str(backups)
    compose("down", "--volumes", "--remove-orphans")
    try:
        print("\n== 1. A fresh platform with demo data")
        compose("up", "--detach", "--build")
        wait_until_up()

        print("\n== 2. Some activity: a purchase")
        student = Client().login(STUDENT)
        _, products = student.call("GET", "/api/shop/products")
        stickers = next(p for p in products if p["name"] == "Lot de stickers")
        status, order = student.buy(stickers["id"])
        assert status == 201 and order["status"] == "pending", (status, order)

        before = snapshot()
        audit_ids = {e["id"] for e in before.pop("audit_before")}

        print("\n== 3. Backup")
        run("scripts/backup.sh", "--with-secrets")
        backup = max(backups.iterdir())
        wait_until_up()

        print("\n== 4. Changes after the backup, then a blank machine")
        admin = Client().login(ADMIN)
        status, _ = admin.call(
            "POST",
            "/api/admin/users",
            {"email": "apres.sauvegarde@epitech.eu", "first_name": "Après",
             "last_name": "Sauvegarde", "role": "student"},
        )
        assert status == 201
        compose("down", "--volumes")

        print("\n== 5. Restore")
        run("scripts/restore.sh", str(backup))
        wait_until_up()

        print("\n== 6. Checks")
        after = snapshot()
        after_audit = after.pop("audit_before")
        for key in before:
            assert before[key] == after[key], f"{key} differs after the restore"
        assert "apres.sauvegarde@epitech.eu" not in after["users"]
        # The trail is the backup's, plus the logins of this check.
        assert audit_ids <= {e["id"] for e in after_audit}
        print("   state identical to the backup: height, state root, balances, orders, users")

        # The restored keys still work: a new payment is signed and included.
        student = Client().login(STUDENT)
        status, order = student.buy(stickers["id"])
        assert status == 201 and order["status"] == "pending", (status, order)
        _, wallet = student.call("GET", "/api/me/wallet")
        assert wallet["balance"] == before["wallet"]["balance"] - stickers["price"]
        print("   the restored platform signs and confirms new payments")

        compose(
            "exec", "-T", "node", "epicoin-cli", "verify",
            "--genesis", "/chain/genesis.json", "--node", "http://127.0.0.1:7400",
        )
        print("   BACKUP AND RESTORE: OK")

        print("\n== 7. Master-key rotation")
        read_key = [
            "docker", "compose", "--profile", "tools", "run", "--rm", "--no-deps", "-T",
            "tools", "sh", "-c", "sed -n 's/^EPICOIN_MASTER_KEY=//p' /secrets/api/keys.env",
        ]
        old_key = subprocess.run(read_key, cwd=REPO, env=ENV, check=True,
                                 capture_output=True, text=True).stdout.strip()
        balance = student.call("GET", "/api/me/wallet")[1]["balance"]
        run("scripts/rotate-master-key.sh")
        wait_until_up()
        new_key = subprocess.run(read_key, cwd=REPO, env=ENV, check=True,
                                 capture_output=True, text=True).stdout.strip()
        assert len(new_key) == 64 and new_key != old_key, "the master key did not change"
        # Every wallet was re-encrypted: a staff member and a student can still sign.
        student = Client().login(STUDENT)
        _, me = student.call("GET", "/api/auth/me")
        staff = Client().login("encadrant1.demo@epitech.eu")
        status, batch = staff.call(
            "POST", "/api/staff/distributions",
            {"recipient_ids": [me["user"]["id"]], "amount": 1000, "reason_code": "event"},
        )
        assert status == 200 and batch["status"] == "included", (status, batch)
        status, order = student.buy(stickers["id"])
        assert status == 201 and order["status"] == "pending", (status, order)
        expected = balance + 1000 - stickers["price"]
        assert student.call("GET", "/api/me/wallet")[1]["balance"] == expected
        print("   MASTER-KEY ROTATION: OK")
        print("\nOPERATIONS: OK")
    finally:
        if os.environ.get("KEEP") != "1":
            compose("down", "--volumes", "--remove-orphans")


if __name__ == "__main__":
    sys.exit(main())
