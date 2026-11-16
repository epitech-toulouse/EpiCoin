# EpiCoin

Monnaie virtuelle interne d'un campus Epitech, sur une blockchain privée en Rust (Proof of
Authority). Les encadrants distribuent des EPC, les étudiants les dépensent dans une boutique.
Environ 150 étudiants et 10 encadrants : la priorité est la simplicité et la robustesse, pas
l'échelle.

La spécification d'origine est résumée ici ; les choix techniques non triviaux sont consignés
dans `docs/DECISIONS.md` (à compléter à chaque nouvelle décision).

## Architecture

```
crates/
  epicoin-core/   règles de la chaîne : types, crypto, transactions, blocs, machine d'état (lib pure, sans I/O)
  epicoin-node/   nœud : mempool, production de blocs, stockage redb, API HTTP interne, mode réplica
  epicoin-api/    backend applicatif : auth, comptes, wallets custodiaux, mint, boutique, admin
  epicoin-cli/    outils : genesis, vérification de la chaîne, administration
web/              front React + TypeScript + Vite
docs/             décisions, guides, modèle de menace
```

Flux : le front parle uniquement à `epicoin-api`. L'API détient les wallets (clés chiffrées),
signe les transactions au nom des utilisateurs et les soumet au nœud. Le nœud valide, produit
les blocs et les stocke. Postgres contient tout ce qui est hors chaîne (utilisateurs, sessions,
boutique, audit).

### Règles de la chaîne (`epicoin-core`)

- Toutes les règles de validation sont dans `State::apply_tx` (`state.rs`) ; les règles de bloc
  dans `Chain::apply_block` (`chain.rs`). Producteur et réplicas passent par le même code.
- Tout ce qui est hashé ou signé est sérialisé en **borsh**. Le JSON (serde) ne sert qu'aux vues
  de lecture et au fichier genesis.
- Hash : BLAKE3 avec séparation de domaine (`crypto::domain`). Signatures : ed25519 strict.
- Adresse = hash de la clé publique. Montants en `u64`, 1 EPC = 100 unités, arithmétique
  `checked_*` uniquement, jamais de flottants.
- Le genesis n'est pas un bloc : son hash est le `prev_hash` du bloc 1.
- Types de transaction : `Mint`, `Purchase`, `Refund`, `AuthorityUpdate`, et `Transfer` qui
  existe dans le format mais est toujours rejeté en V1.

### Nœud (`epicoin-node`)

- `node.rs` : un seul `Mutex` protège la tête de chaîne, la mempool et le store. Écriture
  disque d'abord, mémoire ensuite. Aucune section critique ne traverse un `.await`.
- `store.rs` : un fichier redb ; bloc + index + instantané d'état écrits atomiquement.
- `mempool.rs` : validation à l'arrivée contre l'état « en attente » ; non persistée.
- `http.rs` / `client.rs` / `views.rs` : API interne (sans authentification, réseau privé),
  son client et les formes JSON partagées. Les transactions circulent en borsh.
- `replica.rs` : suit un nœud amont par HTTP en revérifiant chaque bloc.
- `verify.rs` : rejeu complet depuis le genesis (utilisé par `epicoin-cli verify`).
- Mode validateur si `EPICOIN_VALIDATOR_KEY` est défini, mode réplica si `--upstream` l'est.

### API (`epicoin-api`)

- `chain.rs` : seul point d'envoi vers la chaîne. Une transaction est écrite dans `chain_txs`
  avant d'être soumise ; `reconcile` la rejoue jusqu'à un statut final. Ne jamais soumettre une
  transaction au nœud sans passer par `ChainService`.
- `auth.rs` : sessions, CSRF, activation, réinitialisation. L'extracteur `CurrentUser` impose
  session valide + jeton CSRF ; chaque handler appelle ensuite `current.require(&[Role::…])`.
- `vault.rs` / `wallets.rs` : clés custodiales chiffrées ; `load_keypair` est le seul endroit où
  une clé privée existe en clair.
- `authority.rs` : aligne les autorités de mint on-chain sur le rôle `staff`.
- `shop/` : catalogue (`products.rs`) et commandes (`orders.rs`). Le paiement suit trois
  temps (SQL : commande + stock + transaction signée ; soumission ; règlement dans
  `orders::settle`, appelé par `ChainService::finalize`). Lire l'en-tête de `orders.rs` avant
  d'y toucher.
- `users.rs`, `mint.rs`, `me.rs`, `admin.rs` : endpoints par rôle. Erreurs en JSON
  `{error, message}` : `error` est un code stable, `message` un texte français pour l'interface.
- Migrations SQL dans `crates/epicoin-api/migrations/`, appliquées au démarrage.
- Les tests d'intégration (`tests/`) utilisent un vrai Postgres (une base jetable par test via
  `#[sqlx::test]`) et un vrai nœud lancé en mémoire.

### Front (`web/`)

- `src/lib/api.ts` : seul point d'accès à l'API (jeton CSRF en mémoire, erreurs typées).
  `src/lib/format.ts` : montants (entiers d'unités, jamais de flottants), dates, libellés.
- `src/auth.tsx` (session), `src/cart.tsx` (panier + clé d'idempotence), `src/App.tsx`
  (routes par rôle), `src/components/ui.tsx` (composants accessibles), `src/pages/<rôle>/`.
- Toute l'interface est en français ; les textes d'erreur viennent de l'API (`message`).
- `e2e/journey.spec.ts` : le parcours complet sur une pile réelle (`e2e/stack.mjs`).

## Invariants à ne jamais casser

- **RGPD** : aucune donnée personnelle ni texte libre sur la chaîne. Uniquement adresses,
  montants, codes d'enum et hashes. Le lien adresse ↔ utilisateur vit seulement dans Postgres.
- **Format gelé** : ne pas réordonner les champs des types borsh, ne pas changer un discriminant
  d'enum ni une constante de `crypto::domain`. Les nouveaux `ReasonCode` s'ajoutent à la fin.
- **Atomicité** : une transaction rejetée ne modifie pas l'état (vérifié par proptest).
- **Masse monétaire** : `somme des soldes == total_minted - total_burned`.
- **Commande ↔ chaîne** : le statut d'une commande ne change que dans `orders::settle` (issue
  d'une transaction on-chain) ou par une transition explicite du gestionnaire. Le stock n'est
  jamais modifié hors d'une transaction SQL qui modifie aussi la commande.
- **Secrets** : une clé privée n'est jamais loggée, jamais sérialisée en JSON, jamais renvoyée
  par une API. `Keypair` n'implémente ni `Serialize` ni un `Debug` lisible. Les jetons (session,
  invitation, réinitialisation) ne passent jamais dans une URL. Le test
  `private_keys_never_appear_in_responses_or_logs` doit rester vert.

## Commandes

```sh
docker compose up -d postgres              # requis par les tests de l'API (port 5433)
cargo build --workspace
cargo test --workspace                     # tests unitaires, d'intégration et proptests
cargo test -p epicoin-core                 # un seul crate
cargo test -p epicoin-core --test state_machine mint   # un fichier / un filtre
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

# Créer une chaîne locale et lancer un nœud
cargo run -p epicoin-cli -- genesis --chain-id epicoin-dev --out-dir ./data/dev
set -a; . ./data/dev/keys.env; set +a
cargo run -p epicoin-node -- --genesis ./data/dev/genesis.json --data-dir ./data/dev/node

# Auditer la chaîne (nœud en marche, ou base d'un nœud arrêté)
cargo run -p epicoin-cli -- verify --genesis ./data/dev/genesis.json --node http://127.0.0.1:7400
cargo run -p epicoin-cli -- verify --genesis ./data/dev/genesis.json --data-dir ./data/dev/node
```

Le dossier `data/` est ignoré par git : il contient des clés secrètes.

```sh
# Front
cd web && npm install
npm run dev                 # http://localhost:5173, relaie /api vers EPICOIN_API_URL (défaut :8090)
npm run lint && npm run build && npm test
npm run test:e2e            # pile complète jetable ; Postgres de docker compose requis
```

```sh
# Lancer l'API en local (nœud démarré comme ci-dessus)
export EPICOIN_DATABASE_URL=postgres://epicoin:epicoin@127.0.0.1:5433/epicoin
export EPICOIN_PUBLIC_URL=http://localhost:8088 EPICOIN_COOKIE_SECURE=false
export EPICOIN_MASTER_KEY=$(cargo run -q -p epicoin-api -- generate-master-key)   # à conserver !
cargo run -p epicoin-api -- create-admin --email moi@epitech.eu --first-name Prenom --last-name Nom
cargo run -p epicoin-api
```

`DATABASE_URL` vaut par défaut `postgres://epicoin:epicoin@127.0.0.1:5433/epicoin` pour les
tests (voir `.cargo/config.toml`).

Rust est installé via Homebrew (pas de `rustup`) : pas de `rust-toolchain.toml`, on suit le
stable courant.

## Conventions

- Code, identifiants, commentaires et commits en **anglais** ; interface et documentation en
  **français**.
- Les tests s'écrivent en même temps que le code. Un comportement par test, nom descriptif.
- Erreurs : `thiserror` dans les bibliothèques, `anyhow` uniquement dans les binaires.
- Pas de `unwrap`/`expect` hors tests, sauf invariant documenté sur place.
- Fin de phase : tests verts, `cargo fmt`, `cargo clippy -- -D warnings`, un commit.
- En cas de choix non couvert par la spec : l'option la plus simple et standard, notée dans
  `docs/DECISIONS.md`.

## Environnement local

Les ports 5432 et 8080 sont souvent déjà pris sur le poste de développement : les ports exposés
par EpiCoin sont configurables et valent par défaut 5433 (Postgres) et 8088 (web).

## Phases

1. Core — fait
2. Nœud, CLI, réplica — fait
3. API : auth, wallets, mint, audit — fait
4. Boutique — fait
5. Front React — fait
6. Déploiement et documentation
