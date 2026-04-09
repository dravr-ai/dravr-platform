---
title: Dravr sur la messagerie
description: Comment Dravr fonctionne sur les plateformes de messagerie — aperçu, commandes et configuration de groupe.
order: 1
---

Dravr se connecte aux applications de messagerie que vous utilisez déjà — pour que vous puissiez interroger votre entraînement, vérifier votre récupération et travailler avec votre entraîneur sans ouvrir une application séparée.

## Canaux disponibles

| Canal | Idéal pour | Nécessite |
|-------|------------|-----------|
| Telegram | Les athlètes qui veulent un chat personnel rapide et fiable | Un compte Telegram |
| WhatsApp | Les athlètes qui communiquent déjà via WhatsApp | Un compte WhatsApp |
| Slack | Les équipes et entraîneurs sur un espace de travail partagé | Un espace Slack avec Dravr ajouté par un admin |
| Discord | Les communautés sportives et serveurs d'entraînement en groupe | Un serveur Discord avec Dravr ajouté |

Consultez les guides spécifiques à chaque canal pour commencer :
- [Connecter Dravr à Telegram](/fr/docs/telegram)
- [Connecter Dravr à WhatsApp](/fr/docs/whatsapp)
- [Connecter Dravr à Slack](/fr/docs/slack)
- [Connecter Dravr à Discord](/fr/docs/discord)

---

## Comment connecter votre compte

Tous les canaux utilisent le même flux OTP (code à usage unique). Le contact du bot ou le nom d'utilisateur est fourni par votre entraîneur ou l'admin de l'espace de travail.

1. Envoyez un message au bot Dravr sur le canal de votre choix.
2. Dravr répond : *« Bonjour ! Pour lier votre compte Dravr, veuillez entrer votre adresse courriel. »*
3. Entrez l'adresse courriel utilisée pour vous connecter à Dravr.
4. Dravr répond : *« J'ai envoyé un code à 6 chiffres à j***@votredomaine.com »*
5. Vérifiez votre courriel et entrez le code à 6 chiffres dans le chat.
6. Dravr répond : *« Votre compte a été lié avec succès ! »*

**Note :** Le code expire dans 10 minutes. Vous avez 3 tentatives. Tapez `cancel` à tout moment pour annuler.

---

## Commandes

Une fois connecté, ces commandes sont disponibles sur tous les canaux.

| Commande | Fonction |
|----------|----------|
| `/help` | Lister toutes les commandes disponibles |
| `/status` | Voir vos fournisseurs connectés, groupes et canal actif |
| `/coach` | Parcourir les entraîneurs disponibles (carte interactive avec boutons) |
| `/coach select <id>` | Choisir un entraîneur — crée automatiquement un groupe si vous n'en avez pas |
| `/coach assign <coach_id> <group_id>` | Réassigner un entraîneur à un groupe spécifique (admins seulement) |
| `/group` | Lister vos groupes (nom, membres, votre rôle) |
| `/group status` | Afficher les statistiques agrégées de votre groupe |
| `/group members` | Lister les membres de votre groupe |
| `/group invite` | Générer un lien d'invitation valide 7 jours — admin/propriétaire seulement |
| `/group leave` | Quitter votre groupe (demande une confirmation) |
| `/logout` | Dissocier ce canal de votre compte Dravr |

---

## Intégration au groupe

### Créer un groupe (pour les entraîneurs et admins)

1. Liez votre compte au canal en utilisant le flux OTP ci-dessus.
2. Tapez `/coach` — Dravr affiche une liste d'entraîneurs IA avec leurs descriptions.
3. Appuyez ou cliquez sur un entraîneur pour le sélectionner. Dravr crée automatiquement un nouveau groupe.
4. Tapez `/group invite` — Dravr répond avec un lien valide 7 jours.
5. Partagez ce lien avec vos athlètes.

### Rejoindre un groupe (en tant qu'athlète)

1. Ouvrez le lien d'invitation envoyé par votre entraîneur. Il s'ouvre dans un navigateur.
2. Connectez-vous à Dravr (ou créez un compte si vous n'en avez pas).
3. Vous êtes maintenant membre du groupe.
4. Liez ensuite votre canal de messagerie en utilisant le flux OTP ci-dessus pour discuter avec Dravr.

---

## Déconnexion

Tapez `/logout` dans n'importe quel canal. Dravr dissociera ce canal de votre compte. Vos données et appartenances aux groupes ne seront pas affectées.

---

## Ce que vous pouvez demander à Dravr

- « Comment était ma charge d'entraînement cette semaine ? »
- « Suis-je assez récupéré pour une séance intense aujourd'hui ? »
- « Quelle est ma tendance de forme sur le dernier mois ? »
- « Montre-moi mes plus longues courses cette année. »
