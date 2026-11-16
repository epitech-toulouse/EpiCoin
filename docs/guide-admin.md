# Guide d'administration

Ce guide couvre l'installation, la vie courante de la plateforme et les procédures
d'exploitation. Toutes les commandes se lancent à la racine du dépôt, sur le serveur.

- [1. Installer](#1-installer)
- [2. La genèse et les clés](#2-la-genèse-et-les-clés)
- [3. Le premier administrateur](#3-le-premier-administrateur)
- [4. Importer les étudiants](#4-importer-les-étudiants)
- [5. Ajouter un encadrant](#5-ajouter-un-encadrant)
- [6. Gérer les comptes](#6-gérer-les-comptes)
- [7. Sauvegarder et restaurer](#7-sauvegarder-et-restaurer)
- [8. Changer la clé maître](#8-changer-la-clé-maître)
- [9. Vérifier la chaîne](#9-vérifier-la-chaîne)
- [10. Mettre à jour, surveiller, dépanner](#10-mettre-à-jour-surveiller-dépanner)

## 1. Installer

### Prérequis

- Un serveur Linux avec Docker 24+ et Docker Compose v2 (2 Go de RAM suffisent).
- Un nom de domaine et un reverse proxy HTTPS (nginx, Caddy, Traefik… celui du campus convient).
- Un compte SMTP pour envoyer les invitations.

### Configuration

```sh
git clone <dépôt> epicoin && cd epicoin
cp .env.example .env
chmod 600 .env
```

Dans `.env`, adaptez au minimum :

| Variable | Valeur |
|---|---|
| `EPICOIN_PUBLIC_URL` | l'adresse HTTPS du site, par exemple `https://epicoin.campus.epitech.eu` |
| `EPICOIN_COOKIE_SECURE` | `true` |
| `EPICOIN_DEMO_SEED` | `false` — sinon des comptes au mot de passe public seraient créés |
| `EPICOIN_SMTP_URL`, `EPICOIN_MAIL_FROM` | votre serveur SMTP |
| `EPICOIN_POSTGRES_PASSWORD` | un mot de passe long et aléatoire |
| `EPICOIN_CHAIN_ID` | un identifiant propre au campus, par exemple `epicoin-lyon` |

Les autres variables sont commentées dans `.env.example`. `EPICOIN_CHAIN_ID` et le mot de
passe Postgres ne sont lus qu'au premier démarrage.

### Démarrage

```sh
docker compose up -d --build
docker compose ps            # tous les services « healthy », init et seed « exited (0) »
```

Le service `mailpit` (boîte mail de démonstration) démarre aussi ; il est inoffensif (écoute
locale uniquement) et ne reçoit rien quand un vrai SMTP est configuré.

### Reverse proxy

Le conteneur `web` sert le site et relaie `/api` vers l'API. Le reverse proxy n'a qu'à
renvoyer tout le trafic vers lui. Exemple avec nginx sur la même machine :

```nginx
server {
    listen 443 ssl http2;
    server_name epicoin.campus.epitech.eu;
    # ssl_certificate … ; ssl_certificate_key … ;
    add_header Strict-Transport-Security "max-age=31536000" always;
    client_max_body_size 3m;

    location / {
        proxy_pass http://127.0.0.1:8088;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto https;
    }
}
```

`EPICOIN_TRUSTED_PROXIES` doit désigner l'adresse d'où ce proxy joint le conteneur (avec un
proxy sur la même machine, la passerelle Docker : `172.16.0.0/12` convient). Sans ce réglage,
tous les utilisateurs semblent venir de la même adresse et partagent la limite de tentatives
de connexion.

## 2. La genèse et les clés

Au premier démarrage, le service `init` crée la chaîne :

- le **fichier de genèse** (`genesis.json`, public) fixe une fois pour toutes l'identifiant de
  la chaîne, le validateur, le compte administrateur, la trésorerie de la boutique et la durée
  des périodes de quota (30 jours). Le changer revient à créer une autre chaîne ;
- les **clés secrètes**, rangées dans deux volumes séparés :

| Clé | Rôle | Qui la détient |
|---|---|---|
| Validateur | signe les blocs | le nœud uniquement |
| Administration | nomme et révoque les encadrants sur la chaîne | l'API |
| Trésorerie | signe les remboursements de la boutique | l'API |
| Clé maître | chiffre les clés des portefeuilles des utilisateurs | l'API |

**Ces clés ne se régénèrent pas.** Sauvegardez-les dès le premier démarrage, et gardez cette
copie hors ligne (coffre, clé USB chiffrée) :

```sh
scripts/backup.sh --with-secrets
```

Le démarrage suivant détecte la genèse existante et ne recrée rien.

> Pour fournir vos propres clés (générées hors ligne avec `epicoin-cli genesis`), définissez les
> variables `EPICOIN_*_KEY` dans `.env` : elles l'emportent sur les volumes. Le fichier de genèse
> correspondant doit alors être copié dans le volume `chain-config` avant le premier démarrage.

## 3. Le premier administrateur

Il n'existe aucun compte par défaut. Créez le premier administrateur en ligne de commande :

```sh
docker compose run --rm --no-deps api epicoin-api create-admin \
  --email prenom.nom@epitech.eu --first-name Prénom --last-name Nom
```

La commande affiche un lien d'activation à usage unique, valable 7 jours : ouvrez-le pour
choisir le mot de passe. Les administrateurs suivants se créent depuis l'interface.

## 4. Importer les étudiants

Dans **Utilisateurs → Importer un CSV**, envoyez un fichier de ce format (virgule ou
point-virgule, en-têtes en français ou en anglais) :

```csv
email;prenom;nom;promo;role
ada.lovelace@epitech.eu;Ada;Lovelace;2027;etudiant
alan.turing@epitech.eu;Alan;Turing;;encadrant
```

- `promo` et `role` sont facultatifs (rôle par défaut : étudiant). Rôles acceptés : `etudiant`
  (`student`), `encadrant` (`staff`), `boutique` (`shop_manager`), `admin`.
- Seules les adresses du domaine autorisé (`EPICOIN_ALLOWED_EMAIL_DOMAIN`) sont acceptées.
- **Tout ou rien** : si une ligne est invalide, rien n'est importé et chaque erreur est listée
  avec son numéro de ligne. Les adresses déjà inscrites sont simplement ignorées : on peut
  réimporter un fichier complété.
- Chaque nouveau compte reçoit un email d'invitation. Le lien est valable 7 jours ; s'il a
  expiré, ouvrez l'utilisateur et cliquez sur **Renvoyer l'invitation**. Un utilisateur qui
  utilise « Mot de passe oublié » avant d'avoir activé son compte reçoit aussi une invitation.

## 5. Ajouter un encadrant

Créez le compte (import ou **Ajouter un utilisateur**) avec le rôle **Encadrant**, ou changez
le rôle d'un compte existant.

- L'API inscrit aussitôt l'encadrant sur la chaîne comme **autorité de distribution**. Si le
  nœud était indisponible à ce moment-là, la synchronisation automatique (toutes les
  5 minutes) le rattrape.
- Son **quota** est le quota par défaut (`EPICOIN_DEFAULT_MINT_QUOTA_EPC`, 500 EPC par période
  de 30 jours). Pour un quota personnel, ouvrez l'utilisateur et renseignez le champ quota ;
  laissez-le vide pour revenir au quota par défaut. La période est commune à toute la chaîne ;
  le quota non utilisé n'est pas reporté.
- Pour retirer le droit de distribuer : changez le rôle, ou désactivez le compte. La révocation
  est inscrite sur la chaîne.

Un administrateur ne distribue pas lui-même : s'il encadre aussi, il lui faut un second compte
encadrant. La gestion de la boutique, en revanche, lui est ouverte.

## 6. Gérer les comptes

- **Désactiver** un compte : connexion impossible, sessions fermées immédiatement, droit de
  distribuer retiré. Réversible.
- **Supprimer** un compte (droit à l'effacement) : nom, email, promo, sessions et clé du
  portefeuille sont effacés de la base. La chaîne garde l'adresse et ses mouvements, que plus
  rien ne relie à une personne. Les EPC restants deviennent inutilisables. Irréversible.
- Le **journal d'audit** (menu Audit) trace connexions, imports, changements de rôle et de
  quota, distributions, remboursements et suppressions, avec leur auteur et leur date. Il ne
  contient que des identifiants, jamais de nom ni d'email.

## 7. Sauvegarder et restaurer

### Ce qui est sauvegardé

```sh
scripts/backup.sh                  # la base, la chaîne, la genèse, les images des produits
scripts/backup.sh --with-secrets   # la même chose, plus les clés (archive séparée)
```

Chaque sauvegarde est un dossier `backups/<date UTC>/` :

| Fichier | Contenu |
|---|---|
| `postgres.dump` | toute la base (comptes, boutique, audit, clés des portefeuilles **chiffrées**) |
| `chain.tar.gz` | la chaîne, son fichier de genèse, les images des produits |
| `secrets.tar.gz` | avec `--with-secrets` : les clés du validateur, de l'administration, de la trésorerie et la clé maître |
| `SHA256SUMS` | empreintes, vérifiées avant toute restauration |

Le nœud est arrêté pendant les quelques secondes de la sauvegarde, pour que la base et la
chaîne soient prises au même instant. Le site reste en ligne : un paiement lancé à ce moment
attend et se termine dès le redémarrage du nœud.

### Organisation recommandée

- **Les données**, chaque nuit, avec une copie hors du serveur (`EPICOIN_BACKUP_DIR` peut
  pointer vers un montage distant). Elles contiennent des données personnelles : stockez-les
  chiffrées. Sans les clés, elles ne permettent pas de dépenser les EPC.

  ```cron
  30 3 * * * cd /srv/epicoin && scripts/backup.sh >>/var/log/epicoin-backup.log 2>&1
  ```

- **Les clés**, une fois au premier démarrage puis après chaque rotation, **hors ligne**.
  Quiconque obtient `secrets.tar.gz` contrôle la plateforme.

### Restaurer

Sur le même serveur ou sur un nouveau (après `git clone` et la copie du `.env`) :

```sh
scripts/restore.sh backups/20261116T033000Z
```

Le script vérifie les empreintes, arrête la plateforme, **remplace** le contenu de tous les
volumes, **revérifie toute la chaîne depuis sa genèse**, restaure la base, puis redémarre. Les
clés viennent de la sauvegarde si elle contient `secrets.tar.gz`, sinon celles en place sont
conservées (pour restaurer des données sans les clés, restaurez d'abord une sauvegarde de clés).

Les paiements ou distributions qui étaient en cours au moment de la sauvegarde sont repris
automatiquement après la restauration.

### Tester la procédure

```sh
python3 scripts/test-operations.py
```

Le test monte une plateforme jetable, y fait un achat, la sauvegarde, détruit tous ses volumes,
la restaure, vérifie que tout est identique (hauteur, racine d'état, soldes, commandes,
utilisateurs) et que la plateforme signe encore de nouveaux paiements, puis teste la rotation de
la clé maître. Lancez-le après chaque mise à jour de la procédure.

## 8. Changer la clé maître

La clé maître chiffre les clés des portefeuilles. Changez-la si elle a pu fuiter, si une
personne qui y avait accès quitte l'équipe, ou par principe chaque année :

```sh
scripts/rotate-master-key.sh
```

Le script sauvegarde tout (avec l'ancienne clé), arrête l'API, rechiffre chaque portefeuille
sous une nouvelle clé aléatoire — tout ou rien —, installe la nouvelle clé, redémarre l'API puis
sauvegarde les nouvelles clés. Les clés des portefeuilles, donc les adresses et les soldes, ne
changent pas. Détruisez ensuite les archives de clés qui contiennent l'ancienne clé maître.

**Si la clé maître est fournie par `.env`** (et non par le volume), procédez à la main :

1. `scripts/backup.sh --with-secrets`
2. Générez une clé : `docker compose run --rm --no-deps api epicoin-api generate-master-key`
3. Dans `.env` : la nouvelle clé dans `EPICOIN_MASTER_KEY`, l'ancienne dans
   `EPICOIN_MASTER_KEY_PREVIOUS`.
4. `docker compose stop api && docker compose run --rm --no-deps api epicoin-api rotate-master-key`
5. Videz `EPICOIN_MASTER_KEY_PREVIOUS`, puis `docker compose up -d api`.

En cas d'interruption, la commande peut être relancée : elle ne rechiffre que ce qui ne l'est
pas encore.

Les clés de la chaîne (validateur, administration, trésorerie) sont inscrites dans la genèse :
elles ne se changent pas sans créer une nouvelle chaîne. Protégez leurs sauvegardes en
conséquence.

## 9. Vérifier la chaîne

Rejouer toute la chaîne depuis la genèse, en revérifiant hashes, signatures, nonces, quotas,
soldes et racines d'état :

```sh
docker compose exec node epicoin-cli verify --genesis /chain/genesis.json --node http://127.0.0.1:7400
```

Pour un contrôle permanent, lancez un **réplica** : un second nœud, en lecture seule, qui
télécharge chaque bloc et le revérifie avant de le stocker. Il s'arrête au premier bloc invalide.

```sh
docker compose --profile replica up -d replica
docker compose logs -f replica
```

Un réplica sur une autre machine utilise la même image avec `--upstream http://<nœud>:7400` et
le même `genesis.json` ; l'API du nœud n'étant pas authentifiée, ouvrez-la par un VPN ou un
tunnel, jamais sur Internet.

L'explorateur (menu **Blockchain**) montre blocs, transactions et soldes par adresse.

## 10. Mettre à jour, surveiller, dépanner

### Mettre à jour

```sh
scripts/backup.sh
git pull
docker compose up -d --build
```

Les migrations de la base s'appliquent au démarrage de l'API.

### Surveiller

- `docker compose ps` : chaque service a un contrôle de santé.
- `https://<site>/api/health` répond `200` avec `{"status":"ok","database":true,"node":true}`,
  `503` si la base ou le nœud ne répond pas.
- `docker compose logs -f api node` : en production, une ligne JSON par événement
  (`EPICOIN_LOG_FORMAT=json`), prête pour un collecteur. Aucune clé, aucun mot de passe ni aucun
  jeton n'y figure ; ce point est vérifié par les tests.

### Problèmes courants

| Symptôme | Cause probable | Que faire |
|---|---|---|
| Un encadrant voit « autorisation pas encore active » | le nœud était indisponible lors de sa nomination | attendre 5 minutes (synchronisation automatique) ; vérifier `docker compose ps node` |
| Les invitations n'arrivent pas | SMTP mal configuré | `docker compose logs api | grep -i mail` ; vérifier `EPICOIN_SMTP_URL` ; renvoyer l'invitation |
| « La blockchain est momentanément indisponible » | le nœud est arrêté ou en redémarrage | `docker compose up -d node` ; les opérations en attente reprennent seules |
| Une commande reste « Paiement en cours » | idem | elle se confirme ou est annulée (stock rendu) automatiquement au retour du nœud |
| Tous les utilisateurs bloqués « trop de tentatives » | `EPICOIN_TRUSTED_PROXIES` ne correspond pas au reverse proxy | corriger la variable, `docker compose up -d web` |
| Le nœud refuse de démarrer (« snapshot/blocks mismatch ») | fichier de la chaîne endommagé | restaurer la dernière sauvegarde (§ 7) |
