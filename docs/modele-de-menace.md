# Modèle de menace

Version succincte, à relire à chaque évolution notable (transferts entre étudiants, plusieurs
validateurs, connexion Microsoft 365…).

## Ce qu'on protège

| Bien | Pourquoi |
|---|---|
| Le registre des EPC | personne ne doit créer, dépenser ou effacer des EPC en dehors des règles |
| Les clés des portefeuilles | elles permettent de dépenser les EPC d'un utilisateur |
| La clé maître | elle déchiffre toutes les clés des portefeuilles |
| Les clés de la chaîne | administration (nommer des encadrants), trésorerie (rembourser), validateur (produire les blocs) |
| Les données personnelles | nom, prénom, email, promo, historique d'achats |
| Les comptes | sessions, mots de passe |

## Qui pourrait attaquer

- Un **étudiant** qui voudrait plus d'EPC, ou dépenser ceux d'un autre.
- Un **encadrant** qui abuserait de son droit de distribuer, ou dont le compte serait volé.
- Une **personne extérieure** : vol de mots de passe, attaque du site, accès à une sauvegarde.
- Une **personne ayant accès au serveur** (administrateur système, compromission de la machine).

## Menaces et parades

| Menace | Parade |
|---|---|
| Créer des EPC sans y être autorisé | La machine d'état n'accepte un `Mint` que signé par une autorité active, dans la limite de son quota de la période ; l'API ne l'expose qu'au rôle encadrant. Vérifié jusqu'au niveau de la chaîne (une transaction forgée avec la clé d'un étudiant est refusée). |
| Dépenser deux fois, rejouer une transaction | Nonce par compte, signature liée à l'identifiant de la chaîne. Côté API, chaque transaction est enregistrée avant envoi et renvoyée à l'identique : un nouvel essai ne peut pas s'appliquer deux fois. Clés d'idempotence sur les paiements et les distributions. |
| Altérer l'historique | Blocs chaînés par hash, signés par le validateur, racine de Merkle des transactions, racine de l'état. `epicoin-cli verify` et les réplicas rejouent tout depuis la genèse ; toute modification est détectée (testé, y compris sur une chaîne de 10 000 transactions). |
| Encadrant abusif | Quota par période inscrit sur la chaîne ; journal d'audit ; statistiques par encadrant ; révocation immédiate. |
| Vol de mot de passe, force brute | Argon2id, limitation des tentatives par compte et par adresse, messages identiques que le compte existe ou non, sessions révocables, toutes les sessions fermées à chaque changement de mot de passe. |
| Vol de session, requêtes forgées depuis un autre site | Cookie `HttpOnly`, `Secure`, `SameSite=Lax` ; jeton anti-CSRF exigé sur toute requête modifiante ; en-têtes CSP, `X-Frame-Options`, `nosniff`. |
| Interception d'un lien d'activation ou de réinitialisation | Jetons aléatoires de 256 bits, à usage unique, expirants, stockés hashés ; transmis dans le fragment d'URL, jamais dans les journaux. |
| Fuite de la base ou d'une sauvegarde de données | Clés des portefeuilles chiffrées (XChaCha20-Poly1305) avec une clé maître qui n'est jamais en base ; sessions et jetons stockés hashés ; mots de passe en Argon2id. Les sauvegardes de clés sont séparées des sauvegardes de données. |
| Copier la clé chiffrée d'un riche sur son propre compte (accès en écriture à la base) | Chaque clé chiffrée est liée à l'identifiant de son propriétaire : déplacée, elle ne se déchiffre plus. |
| Fuite de secrets dans les journaux ou les réponses | Les types secrets n'ont ni sérialisation ni affichage lisible ; un test parcourt toute l'API et vérifie qu'aucune clé, aucun mot de passe ni aucun jeton n'apparaît dans une réponse ou un journal. |
| Fichier malveillant envoyé comme image de produit | Format reconnu par le contenu (PNG, JPEG, WebP), nom choisi par le serveur, taille limitée, `nosniff`. |
| Données personnelles exposées par la chaîne | Aucune donnée personnelle ni texte libre sur la chaîne (vérifié par des tests qui inspectent les blocs) ; la référence de commande est un hash opaque. Effacement d'un utilisateur possible dans la base. |

## Risques acceptés

Ces risques découlent de choix volontaires de simplicité ; ils sont connus et assumés pour
une plateforme de campus.

1. **Portefeuilles custodiaux.** L'API signe au nom des utilisateurs : qui contrôle l'API (ou la
   clé maître et la base) peut dépenser n'importe quel solde. Les étudiants n'ont aucune clé à
   gérer, c'est l'objectif.
2. **Clé d'administration en ligne.** L'API détient la clé qui nomme les encadrants : une
   compromission de l'API permet de créer une autorité et donc de distribuer sans limite.
   Ce serait visible dans l'explorateur, l'audit et les statistiques. Évolution possible, non
   réalisée en V1 : retirer cette clé de l'API et faire les nominations hors ligne avec
   `epicoin-cli authority grant`, au prix d'une manipulation à chaque nouvel encadrant.
3. **Un seul validateur.** S'il est arrêté, plus aucune transaction n'est confirmée (le site
   reste consultable, les opérations attendent). Un validateur compromis peut refuser des
   transactions ou réécrire la fin de la chaîne, mais pas inclure de transaction invalide :
   les réplicas le détecteraient. Le format permet d'ajouter des validateurs plus tard.
4. **Pas de second facteur.** La sécurité d'un compte repose sur son mot de passe et sur sa
   boîte mail (réinitialisation). Le futur login Microsoft 365 apportera l'authentification
   forte du campus.
5. **Une seule instance de l'API.** La limitation des tentatives et l'ordre des nonces sont
   tenus en mémoire ; un redémarrage remet les compteurs à zéro.
6. **API du nœud sans authentification.** Elle ne doit être joignable que sur le réseau privé
   (c'est le cas avec `docker compose`). Elle n'accepte que des transactions correctement
   signées, mais pourrait être saturée par quelqu'un qui y aurait accès.
7. **Les EPC d'un compte supprimé sont perdus.** La clé du portefeuille est effacée avec les
   données personnelles.
8. **Données de démonstration.** Avec `EPICOIN_DEMO_SEED=true`, des comptes au mot de passe
   public sont créés. Le modèle `.env.example` le désactive ; ne jamais l'activer en production.

## Ce qui reste à la charge de l'exploitation

- HTTPS de bout en bout jusqu'au reverse proxy, en-tête HSTS.
- Mises à jour du système et des images Docker.
- Accès au serveur limité et journalisé.
- Sauvegardes chiffrées et hors site ; sauvegarde des clés hors ligne.
- Lecture régulière du journal d'audit et des statistiques par encadrant.
