# Guide de l'encadrant

En tant qu'encadrant, vous récompensez les étudiants en EpiCoins (EPC). Ils les dépensent
ensuite dans la boutique du campus.

## Se connecter

Vous avez reçu un email « Bienvenue sur EpiCoin ». Ouvrez le lien, choisissez un mot de passe
(10 caractères au moins), et vous voilà connecté. Le lien est valable 7 jours ; passé ce délai,
utilisez « Mot de passe oublié » sur la page de connexion pour en recevoir un nouveau.

## Distribuer des EPC

Menu **Distribuer**.

1. **Choisissez les étudiants.**
   - Cherchez par nom, prénom ou email, filtrez par cohorte (PGE 1, MSc 2027, Bachelor 2…), et
     cochez.
   - **Tout sélectionner** coche tous les étudiants affichés : pratique avec le filtre de
     cohorte.
   - **Importer un fichier CSV** sélectionne d'un coup les étudiants d'une liste : n'importe quel
     fichier contenant des adresses email convient (export d'un tableur, liste d'appel…). Les
     adresses inconnues sont signalées.
2. **Indiquez le montant par étudiant**, en EPC (`5`, `2,50`…). Le total s'affiche.
3. **Choisissez le motif** : Participation, Entraide, Projet remarquable, Événement, Vie du
   campus, Défi ou Autre.
4. **Ajoutez un commentaire** si vous le souhaitez : l'étudiant le lit dans son historique.
5. Cliquez sur **Distribuer**. La confirmation arrive en quelques secondes, le temps que la
   distribution soit inscrite sur la blockchain.

Un double clic, ou un nouvel essai après une coupure réseau, ne distribue jamais deux fois.

## Votre quota

En haut de la page : votre quota restant, ce que vous avez déjà distribué et la date de la
prochaine remise à zéro. Le quota est fixé par période de 30 jours (500 EPC par défaut) ; ce qui
n'est pas distribué n'est pas reporté. Une distribution qui dépasserait votre quota restant est
refusée en entier : personne n'est crédité. Pour un quota plus élevé, adressez-vous à un
administrateur.

## Vos distributions

Menu **Mes distributions** : tout ce que vous avez distribué, avec l'étudiant, le motif, le
montant et l'état (**Confirmée**, **En cours**, **Refusée**).

## Proposer un défi

Menu **Défis** → **Nouveau défi**. Les étudiants qui le réussissent reçoivent les EPC tout de
suite, sans distribution à faire.

- **Titre et description** : ce qu'il faut faire. Les étudiants les lisent tels quels.
- **Gain par étudiant**, en EPC. Il est prélevé sur **votre quota** au moment où chaque
  étudiant réussit (motif « Défi »). Si votre quota est épuisé, l'étudiant est prié de
  réessayer plus tard : il n'est pas payé, et n'est pas compté comme gagnant.
- **Validation**, au choix :
  - **Réponse à trouver** : énigme, quiz, code donné en fin d'atelier, flag… L'étudiant saisit
    la réponse ; si elle est juste, il est payé aussitôt. Majuscules et espaces ne comptent
    pas. La réponse n'est plus jamais affichée, même à vous ; vous pouvez la remplacer en
    modifiant le défi. Chaque étudiant a 5 essais toutes les 15 minutes : évitez les réponses
    trop faciles à deviner (un chiffre, oui/non).
  - **Validé par un encadrant** : l'étudiant explique comment il a réussi (un lien, quelques
    lignes). Dans **Participants**, vous validez (il est payé aussitôt) ou refusez avec un
    motif ; il peut alors renvoyer sa participation.
- **Cohortes** (facultatif) : le défi n'est proposé qu'à ces cohortes. Aucune case cochée :
  tous les étudiants.
- **Gagnants maximum** et **date limite** (facultatifs). Vous pouvez aussi clore le défi à la
  main en le modifiant (**Défi terminé**).

Chaque étudiant ne gagne un défi qu'une fois, même s'il clique deux fois. Les administrateurs
voient et peuvent modifier tous les défis.

## Bon à savoir

- Une distribution confirmée est **définitive** : la blockchain ne permet pas de l'annuler. En cas
  d'erreur, prévenez un administrateur.
- La blockchain n'enregistre que des adresses, des montants et le motif. Votre nom, celui de
  l'étudiant et votre commentaire restent dans la base de la plateforme.
- Vous ne pouvez distribuer qu'aux étudiants, et jamais à vous-même.
- Si la page indique que votre autorisation n'est pas encore active, attendez une minute :
  elle est inscrite sur la blockchain peu après votre nomination.
