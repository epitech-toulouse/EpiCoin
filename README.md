# EpiCoin

Monnaie virtuelle interne d'un campus Epitech. Les encadrants récompensent les étudiants en
EpiCoins (EPC), que ceux-ci dépensent dans une petite boutique en ligne.

Les EPC vivent sur une **blockchain privée** écrite en Rust : chaque distribution, achat et
remboursement est une transaction signée, incluse dans un bloc, vérifiable par n'importe qui
à partir du bloc de genèse. La chaîne ne contient **aucune donnée personnelle** : seulement des
adresses, des montants et des codes.

## Essayer en cinq minutes

Il suffit de Docker (avec Docker Compose v2).

```sh
git clone <ce dépôt> epicoin && cd epicoin
docker compose up
```

Le premier démarrage compile les images, crée la chaîne et ses clés, puis la remplit de données
de démonstration. Quand les logs se calment :

| | |
|---|---|
| Le site | http://localhost:8088 |
| La boîte mail de démo (invitations, réinitialisations) | http://localhost:8025 |

Comptes de démonstration, tous avec le mot de passe **`epicoin-demo`** :

| Rôle | Email |
|---|---|
| Administrateur | `admin.demo@epitech.eu` |
| Boutique | `boutique.demo@epitech.eu` |
| Encadrants | `encadrant1.demo@epitech.eu` … `encadrant5.demo@epitech.eu` |
| Étudiant (un parmi 150) | `camille.martin@epitech.eu` |

Pour tout effacer et repartir de zéro : `docker compose down --volumes`.

## Installer pour de vrai

Lisez le [guide d'administration](docs/guide-admin.md). En résumé : un fichier `.env` tiré de
[`.env.example`](.env.example) (HTTPS, SMTP, **pas de données de démo**), `docker compose up -d`,
la sauvegarde des clés générées, puis la création du premier administrateur.

## Documentation

| Document | Pour qui |
|---|---|
| [Guide d'administration](docs/guide-admin.md) | installation, genèse, import des étudiants, encadrants, sauvegardes, rotation des clés |
| [Guide de l'encadrant](docs/guide-encadrant.md) | distribuer des EPC |
| [Modèle de menace](docs/modele-de-menace.md) | ce qui est protégé, contre quoi, et les risques acceptés |
| [Décisions techniques](docs/DECISIONS.md) | les choix qui ne vont pas de soi, et pourquoi |
| [CLAUDE.md](CLAUDE.md) | architecture et conventions, pour qui modifie le code |

## Architecture

```
navigateur ──HTTPS──▶ reverse proxy ──▶ web (nginx : front React + relais /api)
                                              │
                                              ▼
                                         api (Rust) ──▶ PostgreSQL   comptes, boutique, audit
                                              │                       clés des portefeuilles chiffrées
                                              ▼
                                         node (Rust) ──▶ redb        la blockchain
                                              ▲
                                    replica (optionnel, revérifie chaque bloc)
```

| Dossier | Contenu |
|---|---|
| `crates/epicoin-core` | règles de la chaîne : transactions, blocs, machine d'état (sans I/O) |
| `crates/epicoin-node` | nœud : mempool, production des blocs, stockage, API interne, réplica |
| `crates/epicoin-api` | backend : authentification, portefeuilles, distributions, boutique, administration |
| `crates/epicoin-cli` | genèse, vérification complète de la chaîne, gestion des autorités |
| `web/` | interface React + TypeScript |
| `scripts/` | sauvegarde, restauration, rotation de la clé maître et leur test |

## Développer

```sh
docker compose up -d postgres         # base de test, port 5433
cargo test --workspace                # Rust : unitaires, intégration, proptests
cargo clippy --workspace --all-targets -- -D warnings
cd web && npm install && npm test     # front : tests unitaires
npm run test:e2e                      # parcours complet dans un navigateur
python3 scripts/test-operations.py    # sauvegarde, restauration, rotation (Docker)
```

Le détail des commandes et des conventions est dans [CLAUDE.md](CLAUDE.md).
