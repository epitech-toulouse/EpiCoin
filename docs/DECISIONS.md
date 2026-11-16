# Décisions techniques

Choix non imposés par la spécification, avec leur raison. À compléter au fil des phases.

## Cadre général

| # | Décision | Raison |
|---|---|---|
| G1 | Code et commits en anglais, interface et docs en français. | Validé avec le porteur du projet. |
| G2 | Pas de `rust-toolchain.toml` ; on suit le Rust stable courant. | Rust est installé sans `rustup` sur le poste de dev ; la CI utilise `stable`. |
| G3 | Ports par défaut 5433 (Postgres) et 8088 (web), configurables. | 5432 et 8080 sont déjà occupés sur le poste de dev. |

## Chaîne (`epicoin-core`)

| # | Décision | Raison |
|---|---|---|
| C1 | Hash BLAKE3 partout, avec un contexte de dérivation par usage (`crypto::domain`). | Un seul algorithme ; la séparation de domaine empêche de confondre deux hashes de natures différentes. |
| C2 | Adresse = BLAKE3 (32 octets) de la clé publique ; la transaction porte `from` **et** `pubkey`. | La spec veut une adresse hashée ; il faut donc transporter la clé pour vérifier la signature. On vérifie que `hash(pubkey) == from`. |
| C3 | La signature couvre `chain_id`, `from`, `pubkey`, `nonce`, `payload`. | Le `chain_id` empêche de rejouer une transaction d'une autre chaîne EpiCoin (dev vers prod). |
| C4 | Identifiant de transaction = hash de l'encodage signé complet. | La racine de Merkle engage aussi les signatures. |
| C5 | Vérification ed25519 stricte (`verify_strict`). | Refuse les signatures malléables et les clés faibles. |
| C6 | Le genesis n'est pas un bloc : c'est une configuration dont le hash sert de `prev_hash` au bloc 1. | Évite un bloc 0 avec une signature factice. |
| C7 | `state_root` = hash de l'état complet sérialisé (maps triées). | À quelques centaines de comptes, un arbre de Merkle sur l'état serait de la sur-ingénierie. |
| C8 | Horodatages en millisecondes, strictement croissants d'un bloc au suivant. | Ordre total simple, y compris quand les blocs sont produits rapidement (tests). |
| C9 | Quota de mint par fenêtres de durée fixe comptées depuis le genesis (30 jours par défaut), sans report. | Déterministe au rejeu, sans logique de calendrier ni de fuseau. |
| C10 | Une autorité révoquée reste dans l'état (`active = false`). | Révoquer puis ré-autoriser dans la même période ne remet pas le compteur à zéro. |
| C11 | Un encadrant ne peut pas minter vers lui-même. | Garde-fou élémentaire, une ligne de code. |
| C12 | Remboursement intégral et unique ; l'état garde chaque achat (acheteur, montant, remboursé). | Empêche le double remboursement ; pas de remboursement partiel en V1. |
| C13 | Les EPC dépensés s'accumulent sur le compte trésorerie ; pas de transaction `Burn`. `total_burned` existe et vaut 0. | Validé avec le porteur du projet ; le compteur garde la forme de l'invariant de masse monétaire. |
| C14 | `Transfer` est dans le format mais toujours rejeté (`TransfersDisabled`). | L'activer plus tard ne change pas le format des transactions. |
| C15 | Validateur attendu à la hauteur `h` : `validators[(h - 1) % n]`. | Round-robin prêt pour plusieurs validateurs ; avec `n = 1` c'est toujours le même. |
| C16 | Une distribution groupée = N transactions `Mint`. | Pas de type de transaction supplémentaire. |
| C17 | Motifs (`ReasonCode`) : participation, entraide, projet remarquable, événement, vie du campus, défi, autre. | Liste validée ; extensible en ajoutant à la fin. |
| C18 | Les blocs vides sont valides. | Permet un heartbeat optionnel côté nœud. |
| C19 | La clé admin est détenue en ligne par l'API (chiffrée). | Validé : nommer un encadrant se fait depuis l'interface. Contrepartie notée dans le modèle de menace. |

## Nœud et CLI (`epicoin-node`, `epicoin-cli`)

| # | Décision | Raison |
|---|---|---|
| N1 | Un seul verrou protège la tête de chaîne, la mempool et le stockage ; les sections critiques sont courtes et synchrones. | Ordre total des événements sans subtilité de concurrence ; largement suffisant pour ~150 utilisateurs. |
| N2 | Un bloc, ses index et l'instantané d'état sont écrits dans la même transaction redb. | Après un crash, le fichier contient tout le bloc ou rien. |
| N3 | Au démarrage, le nœud vérifie seulement que l'instantané correspond au dernier bloc (hauteur, hash, `state_root`). L'audit complet est `epicoin-cli verify`. | Démarrage rapide ; un instantané falsifié ou restauré d'une ancienne version est tout de même refusé. |
| N4 | La mempool valide chaque transaction à l'arrivée contre un « état en attente » (état de la chaîne + transactions déjà en file). | Refus immédiat avec la raison exacte ; un émetteur peut enchaîner plusieurs nonces. |
| N5 | La mempool n'est pas persistée. La soumission est idempotente : renvoyer une transaction déjà en file ou déjà incluse retourne son statut. | L'API conserve ses transactions signées et les renvoie après un redémarrage, sans risque de double effet. |
| N6 | `POST /tx?wait_ms=` attend l'inclusion dans un bloc (30 s max). | Évite à l'API de sonder le nœud. |
| N7 | L'API du nœud n'a pas d'authentification et écoute par défaut sur `127.0.0.1`. | Elle est interne (réseau privé) ; une écriture n'est acceptée qu'avec une signature valide. |
| N8 | Le client HTTP entre services n'embarque pas TLS. | Les services se parlent sur un réseau privé ; un réplica distant passe par un VPN ou un tunnel. |
| N9 | Les clés secrètes (validateur, admin) se passent par variable d'environnement, jamais par option de ligne de commande. | Les options sont visibles dans la liste des processus et l'historique du shell. |
| N10 | `verify --data-dir` exige que le nœud soit arrêté ; `verify --node` audite un nœud en marche. | redb verrouille le fichier pour un seul processus. |
| N11 | Un réplica est le même binaire sans clé de validateur, avec `--upstream`. Il revérifie chaque bloc avant de le stocker et s'arrête au premier bloc invalide. | Un réplica synchronisé est un audit continu de la chaîne. |
| N12 | `epicoin-cli genesis` refuse d'écraser un `genesis.json` ou un `keys.env` existant, et écrit `keys.env` en mode 0600. | Écraser les clés d'une chaîne en service serait irréversible. |
| N13 | Index des transactions par adresse dans redb (`address_txs`). | Nécessaire à l'explorateur ; reconstruit à partir des blocs, donc jamais une source de vérité. |
