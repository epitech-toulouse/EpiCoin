# Décisions techniques

Choix non imposés par la spécification, avec leur raison. À compléter au fil des phases.

## Cadre général

| # | Décision | Raison |
|---|---|---|
| G1 | Code et commits en anglais, interface et docs en français. | Validé avec le porteur du projet. |
| G2 | Pas de `rust-toolchain.toml` ; on suit le Rust stable courant. | Rust est installé sans `rustup` sur le poste de dev ; la CI utilise `stable`. |
| G3 | Ports par défaut 5433 (Postgres), 7400 (nœud), 8090 (API) et 8088 (web), tous configurables. | 5432, 8000 et 8080 sont déjà occupés sur le poste de dev, et macOS réserve 7000 (AirPlay). |

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

## API (`epicoin-api`)

| # | Décision | Raison |
|---|---|---|
| A1 | Toute transaction signée par l'API est enregistrée dans `chain_txs` (avec ses octets signés) **avant** l'envoi au nœud ; un réconciliateur renvoie les mêmes octets jusqu'à inclusion ou refus définitif. | Un retry, un crash ou un redémarrage du nœud ne peut jamais appliquer deux fois une transaction : le nonce ne change pas. |
| A2 | Les nonces sont alloués sous un verrou par signataire, en mémoire. L'API tourne en **une seule instance**. | Simple et suffisant ; plusieurs instances demanderaient un verrou Postgres. |
| A3 | Sessions : jeton aléatoire opaque dans un cookie `HttpOnly` + `SameSite=Lax` (+ `Secure`), stocké hashé. Durée fixe (7 jours par défaut). | Standard, révocable côté serveur ; une fuite de la table ne donne pas de session utilisable. |
| A4 | CSRF par jeton synchronisé : l'en-tête `X-CSRF-Token` est exigé sur toute requête modifiante d'un utilisateur connecté. | Indépendant du comportement `SameSite` des navigateurs. |
| A5 | Les liens envoyés par email portent leur jeton dans le **fragment** d'URL (`/activation#token=…`) et l'API ne reçoit les jetons que dans le corps des requêtes. | Les URL finissent dans les journaux d'accès (nginx, traces) ; le fragment n'est jamais envoyé au serveur. Vérifié par un test. |
| A6 | Limitation de débit en mémoire : 5 tentatives de connexion par compte et 30 par adresse cliente sur 15 minutes ; 3 demandes de réinitialisation par compte par heure. | Suffisant pour une instance unique ; perdre les compteurs au redémarrage est sans gravité. |
| A7 | Mot de passe : 10 caractères minimum, Argon2id avec les paramètres par défaut de la crate, calculé hors des threads asynchrones. | Recommandation OWASP courante ; pas de règles de composition. |
| A8 | Clés des wallets chiffrées en XChaCha20-Poly1305, l'identifiant du propriétaire en donnée associée ; chaque ligne porte l'empreinte de la clé maître utilisée. | Un chiffré copié sur un autre compte ne se déchiffre pas ; la rotation sait quelle clé relire. |
| A9 | Rotation de la clé maître : nouvelle clé dans `EPICOIN_MASTER_KEY`, ancienne dans `EPICOIN_MASTER_KEY_PREVIOUS`, puis `epicoin-api rotate-master-key` (tout ou rien). | Procédure courte, rejouable sans risque. |
| A10 | L'autorité de mint on-chain suit les rôles qui distribuent (`staff` et `admin`, `Role::DISTRIBUTORS`) : Postgres exprime l'intention, `authority::sync_user` l'applique tout de suite et `sync_all` la rattrape toutes les 5 minutes. | Le système converge seul si le nœud était injoignable au moment d'un changement de rôle. |
| A11 | Les rôles `staff` et `admin` distribuent, avec les mêmes règles (autorité on-chain, quota par défaut ou personnel, audit). Révise la première version, où seul `staff` distribuait. | Demande d'usage : un admin qui encadre n'a plus besoin d'un second compte. Il ne gagne pas de pouvoir nouveau : il pouvait déjà nommer des encadrants et fixer leurs quotas. Ses distributions sont signées par son propre wallet et tracées comme les autres. |
| A12 | Une distribution est validée en entier avant tout envoi (destinataires, quota restant) ; l'en-tête `Idempotency-Key` rend un double clic sans effet. | Tout ou rien du point de vue de l'encadrant. |
| A13 | Le motif (`reason_code`) va sur la chaîne ; le commentaire libre reste dans `mint_batches`. | RGPD : aucun texte libre on-chain (vérifié par un test qui inspecte les blocs). |
| A14 | Suppression d'un utilisateur : la ligne `users` est effacée (cascade sur wallet, sessions, jetons) ; `chain_txs` et `audit_log` gardent des références mises à `NULL`. Le journal d'audit conserve l'identifiant opaque de ce qui a été supprimé. | Efface identité et clé privée ; l'identifiant restant ne désigne plus personne et prouve la suppression. Les EPC restants sur l'adresse deviennent indépensables. |
| A15 | Les détails du journal d'audit ne contiennent que des identifiants, des compteurs et des codes. | Pas de donnée personnelle à purger dans l'audit. |
| A16 | Import CSV : tout ou rien si une ligne est invalide ; pour un email déjà connu, seule la cohorte est mise à jour (jamais le nom ni le rôle) ; un fichier `email` + `cohorte` sert à changer des étudiants de cohorte ; invitations et autorités partent en tâche de fond. | Pas d'import à moitié fait ; le passage à l'année suivante se fait par le même écran ; un import ne peut pas élever un rôle ; la requête de l'admin ne dépend pas de 150 envois d'email. |
| A17 | Requêtes `sqlx` vérifiées à l'exécution (pas de macros `query!`) ; migrations embarquées et appliquées au démarrage. | Le projet compile sans base de données ; les requêtes sont couvertes par les tests d'intégration. |
| A18 | Le mailer de développement (`console`) écrit le message complet, lien d'activation compris, dans les logs. | C'est son rôle en développement ; ne jamais l'utiliser en production (`EPICOIN_MAILER=smtp`). |
| A19 | Le premier administrateur se crée en ligne de commande : `epicoin-api create-admin`, qui affiche son lien d'activation. | Pas de compte par défaut ni de mot de passe initial. |
| A20 | Les cohortes forment une liste fermée dans le code (`models::COHORTS`, dans l'ordre d'affichage), servie au front par `/api/staff/promos`. La colonne `users.promo` contient leur code (`pge1`), ou une chaîne vide. La saisie est tolérante (`PGE 1`, `Bachelor 2`). Une ancienne valeur hors liste est conservée, et l'étudiant est classé « sans cohorte ». | La liste change une fois par an (nouvelle année de MSc) : une ligne de code suffit, sans table ni écran d'administration. Filtres et compteurs restent justes, sans fautes de frappe. |

## Boutique (`epicoin-api::shop`)

| # | Décision | Raison |
|---|---|---|
| S1 | Un achat se fait en trois temps : (1) une transaction SQL crée la commande `awaiting_payment`, prend le stock et enregistre le `Purchase` signé ; (2) la transaction est soumise au nœud ; (3) à son statut final, `ChainService::finalize` met à jour la commande **dans la même transaction SQL** que le statut de la transaction. | Postgres et la chaîne ne partagent pas de transaction : cet ordre garantit qu'une rupture de stock annule tout avant débit, et qu'un paiement refusé rend le stock. Un arrêt entre deux étapes est rattrapé par le réconciliateur. |
| S2 | `Idempotency-Key` est **obligatoire** au paiement ; `(utilisateur, clé)` est unique dans `orders`. | Double clic ou retry retrouvent la commande existante : un seul débit. Vérifié y compris pour deux requêtes simultanées. |
| S3 | Deux statuts internes s'ajoutent au cycle de la spec : `awaiting_payment` (paiement enregistré, pas encore confirmé) et `failed` (paiement refusé). L'annulation passe par `refunding` jusqu'à l'inclusion du `Refund`. | Rendent visibles les états intermédiaires au lieu de les cacher ; aucun n'est durable en fonctionnement normal. |
| S4 | Le stock est pris dans la transaction SQL sous `SELECT … FOR UPDATE`, produits triés par identifiant. | Deux acheteurs du dernier article : un seul passe ; pas d'interblocage entre paniers. |
| S5 | L'API vérifie le solde avant de créer la commande, mais c'est la chaîne qui tranche. | Message clair dans le cas courant ; deux commandes simultanées qui dépasseraient le solde sont arrêtées par la mempool. |
| S6 | Un `Purchase` par commande, pour le total du panier ; `order_ref` = hash de l'identifiant de commande. | La chaîne ne révèle ni la commande ni les produits. |
| S7 | Annulation possible depuis `pending` ou `ready`, jamais après `delivered` ; remboursement intégral ; le stock revient à l'inclusion du `Refund`. | Suit la règle on-chain (un seul remboursement, intégral). |
| S8 | Les lignes de commande copient le nom et le prix du produit. Un produit déjà commandé ne peut plus être supprimé, seulement désactivé. | L'historique ne change pas quand le catalogue change. |
| S9 | Images produit : envoi du fichier brut (2 Mo max), format reconnu par ses premiers octets (PNG, JPEG, WebP), nom généré par le serveur, servies sans authentification avec `nosniff`. | Pas de SVG ni de HTML déguisé ; aucun nom de fichier venant du client n'atteint le disque. |
| S10 | Un administrateur peut faire tout ce que fait le gestionnaire de boutique. Seuls les étudiants achètent. | Petite équipe : l'admin dépanne. Les encadrants ne détiennent pas d'EPC. |

## Défis (`epicoin-api::challenges`)

| # | Décision | Raison |
|---|---|---|
| C1 | Deux modes au choix par défi : réponse à trouver (vérifiée par l'API, gain immédiat) ou validation par un encadrant. Créés par les encadrants et les admins ; un admin voit et gère tous les défis. | Couvre les énigmes et quiz (automatique) comme les défis concrets (animer un atelier), sans écran de plus pour l'étudiant. |
| C2 | Le gain est un `Mint` signé par le **créateur** du défi, motif `challenge`, compté sur son quota. Aucune clé ni autorité propre aux défis. | Le format de la chaîne ne change pas ; toute émission reste plafonnée par le quota d'une personne et apparaît dans ses distributions et les statistiques. |
| C3 | Le statut `approved` d'une participation et la transaction de versement sont écrits dans la même transaction SQL, sous le verrou du signataire et un `FOR UPDATE` sur le défi. Un versement refusé par la chaîne rend la participation à nouveau payable. | Un gagnant est payé exactement une fois, même avec deux réponses simultanées (testé) ; la limite de gagnants tient. |
| C4 | Le quota du créateur est vérifié avant tout enregistrement : s'il ne suffit pas, l'étudiant est prié de réessayer et rien n'est écrit. | Pas de « gagnant non payé » à rattraper à la main. |
| C5 | La réponse est stockée en Argon2id, comparée sans tenir compte de la casse ni des espaces, jamais renvoyée par l'API ; 5 essais par étudiant et par défi toutes les 15 minutes (limiteur en mémoire, voir A6). | Une fuite de la base ne livre pas les réponses ; deviner coûte du temps. Une réponse trop simple reste un risque laissé au créateur (le guide le dit). |
| C6 | Chaque gain crée un lot `mint_batches` d'un destinataire, avec le commentaire « Défi : titre ». | L'étudiant lit le nom du défi dans son historique ; le créateur le retrouve dans ses distributions, sans nouvelle vue. |

## Front (`web/`)

| # | Décision | Raison |
|---|---|---|
| F1 | React + TypeScript + Vite, TanStack Query, Tailwind CSS v4, sans bibliothèque de composants : une douzaine de composants maison accessibles dans `components/ui.tsx` (le `<dialog>` natif pour les modales). | shadcn/ui suppose Radix et un générateur de code : trop de dépendances pour une douzaine d'écrans. |
| F2 | TypeScript 6 et non 7. | `typescript-eslint` ne supporte pas encore TypeScript 7. |
| F3 | Le navigateur ne parle qu'à sa propre origine : `/api` est relayé par Vite en dev, par nginx en production. | Cookies `SameSite` et CSRF simples, pas de CORS. |
| F4 | Le jeton CSRF reste en mémoire ; rien de secret n'est stocké par la page. Le panier et la clé d'idempotence du paiement vont dans `sessionStorage`. | Le cookie de session est `HttpOnly` ; le panier survit à un rechargement de l'onglet, pas plus. |
| F5 | Chaque paiement et chaque distribution porte une clé d'idempotence générée côté navigateur, conservée après une erreur réseau et renouvelée après un refus définitif ou une modification. | Un nouvel essai après une coupure ne débite ni ne distribue deux fois. |
| F6 | Les montants sont des entiers d'unités ; `parseEpc` convertit le texte saisi sans passer par les flottants. | Pas d'erreur d'arrondi (`0,1 + 0,2`). |
| F7 | Les données sont rechargées à chaque affichage d'écran (`staleTime: 0`) ; les files de travail se rafraîchissent périodiquement. | Soldes, stock et commandes sont modifiés par d'autres personnes ; un écran ne doit jamais montrer un état périmé. |
| F8 | Toute réponse 401 `unauthorized` ramène à l'écran de connexion et vide les données en mémoire. | Session expirée, désactivée ou fermée dans un autre onglet. |
| F9 | Le jeton d'un lien reçu par email est lu dans le fragment d'URL. | Voir A5. |
| F11 | Design inspiré de l'intranet du campus : barre bleue, menu latéral en sections `< … />`, fond gris à pointillés, cartes carrées, titres et chiffres en police condensée (Anton), libellés en monospace (IBM Plex Mono), texte courant en Inter. Le logo est `{EPICOIN}`, pas celui d'Epitech. | Les utilisateurs retrouvent les codes visuels qu'ils connaissent ; aucune marque reproduite. |
| F12 | Polices hébergées par le site (paquets `@fontsource`), icônes `lucide-react`. | La CSP n'autorise que l'origine du site ; pas de dépendance à un CDN. |
| F13 | Couleurs des graphiques validées pour le daltonisme : vert/ambre (comptes), bleu/sarcelle (masse monétaire). Vert et rouge ne sont jamais côte à côte ; chaque valeur est aussi écrite en clair. Le texte coloré utilise des teintes plus foncées (contraste ≥ 4,5:1). | Lisible par tous, sans dépendre de la seule couleur. |
| F10 | Les tests E2E lancent une pile complète et jetable (`e2e/stack.mjs` : nouvelle chaîne, nouvelle base, nœud, API, front compilé) ; les emails sont lus dans le journal du mailer console. | Le parcours est testé de bout en bout sur les vrais binaires, sans service SMTP. |

## Déploiement et exploitation

| # | Décision | Raison |
|---|---|---|
| D1 | Une image Docker pour les trois binaires Rust, une pour le front (nginx). Le front relaie `/api` vers l'API ; seul le conteneur web publie un port (sur `127.0.0.1` par défaut). | Une seule origine pour le navigateur ; Postgres, l'API et le nœud restent sur le réseau interne. |
| D2 | Sans `.env`, `docker compose up` lance une démo complète (genèse, clés, données de démo, Mailpit). Le modèle `.env.example` décrit une installation réelle et désactive les données de démo. | Répond au « machine vierge → plateforme fonctionnelle » sans risquer qu'une installation réelle hérite de comptes au mot de passe public. |
| D3 | Le premier démarrage génère la genèse et toutes les clés (service `init`, une seule fois). Les clés vont dans deux volumes séparés : le nœud ne voit que la clé du validateur, l'API que les siennes. Une variable d'environnement définie l'emporte sur le volume. | Démarrage sans manipulation, moindre privilège, et possibilité de fournir des clés générées hors ligne. |
| D4 | Les données de démo sont créées par `epicoin-api seed-demo` (service `seed`), avant le démarrage de l'API, et seulement sur une base vide. | Un seul processus signe pendant le seed : pas de conflit de nonce avec l'API. |
| D5 | Sauvegarde : le nœud est arrêté le temps du `pg_dump` et de la copie de la chaîne. | Base et chaîne cohérentes : toute transaction incluse a sa ligne dans la sauvegarde (elle est enregistrée avant envoi) ; les lignes restées « en cours » sont rejouées après restauration. Quelques secondes de confirmations différées, pas de coupure du site. |
| D6 | Les clés sont sauvegardées à part (`--with-secrets`), à garder hors ligne ; les sauvegardes de données peuvent partir hors site. | Une sauvegarde de données volée ne permet pas de dépenser les EPC. |
| D7 | La restauration revérifie toute la chaîne avant de redémarrer. | On ne remet jamais en service une chaîne altérée. |
| D8 | Rotation de la clé maître par script (sauvegarde, rechiffrement tout-ou-rien, installation de la nouvelle clé, nouvelle sauvegarde). | Procédure rare, donc à automatiser plutôt qu'à se rappeler. |
| D9 | Sauvegarde, restauration et rotation sont testées de bout en bout par `scripts/test-operations.py` (plateforme jetable, destruction des volumes entre sauvegarde et restauration). | « Procédure de restauration testée » au sens strict. |
| D10 | Limites par adresse cliente relevées à 200 connexions et 30 demandes de réinitialisation par fenêtre ; nginx retrouve l'adresse réelle derrière un proxy de confiance (`EPICOIN_TRUSTED_PROXIES`). | Tout un campus peut sortir par la même adresse (NAT) ; la vraie protection est la limite par compte. |
| D11 | Mailpit est inclus dans le compose comme boîte mail de démonstration (écoute locale uniquement). | La démo montre les vrais emails, envoyés par le vrai mailer SMTP. |
| D12 | Journaux JSON par défaut dans le compose ; contrôles de santé sur chaque service ; `/api/health` vérifie base et nœud. | Exploitation standard, prête pour un collecteur. |
