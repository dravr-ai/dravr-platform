---
title: Connecter à Telegram
description: Liez votre compte Dravr à Telegram pour un accès rapide et privé à vos données d'entraînement.
order: 4
platform: telegram
---

Telegram est idéal pour les athlètes qui veulent un accès rapide et fiable à Dravr via une application de chat légère. Les conversations sont privées par défaut et fonctionnent bien sur téléphone comme sur ordinateur.

## Avant de commencer

- Vous avez besoin d'un compte Dravr. Si vous n'en avez pas, demandez à votre entraîneur ou à l'admin.
- Votre entraîneur ou admin doit vous fournir le nom d'utilisateur du bot Dravr sur Telegram.

## Comment se connecter

1. Dans Telegram, recherchez le bot Dravr par son nom d'utilisateur (fourni par votre entraîneur ou admin).
2. Envoyez n'importe quel message (par exemple, « Bonjour ») pour démarrer la conversation.
3. Dravr répond : *« Bonjour ! Pour lier votre compte Dravr, veuillez entrer votre adresse courriel. »*
4. Entrez l'adresse courriel utilisée pour vous connecter à Dravr.
5. Dravr répond : *« J'ai envoyé un code à 6 chiffres à j***@votredomaine.com »*
6. Vérifiez votre courriel et entrez le code à 6 chiffres dans le chat Telegram.
7. Dravr répond : *« Votre compte a été lié avec succès ! »*

**Note :** Le code expire dans 10 minutes. Vous avez 3 tentatives. Tapez `cancel` à tout moment pour annuler.

---

## Commandes

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
| `/logout` | Dissocier Telegram de votre compte Dravr |

---

## Intégration au groupe

### Créer un groupe (pour les entraîneurs et admins)

1. Liez votre compte en suivant les étapes ci-dessus.
2. Tapez `/coach` — Dravr affiche une liste d'entraîneurs IA avec leurs descriptions.
3. Cliquez sur un entraîneur pour le sélectionner. Dravr crée automatiquement un nouveau groupe.
4. Tapez `/group invite` — Dravr répond avec un lien valide 7 jours.
5. Partagez ce lien avec vos athlètes.

### Rejoindre un groupe (en tant qu'athlète)

1. Ouvrez le lien d'invitation envoyé par votre entraîneur. Il s'ouvre dans un navigateur.
2. Connectez-vous à Dravr (ou créez un compte).
3. Vous êtes maintenant membre du groupe.
4. Suivez ensuite les étapes de connexion ci-dessus pour lier Telegram et discuter avec Dravr.

---

## Ce que vous pouvez demander à Dravr

- « Comment était ma charge d'entraînement cette semaine ? »
- « Suis-je assez récupéré pour une séance intense aujourd'hui ? »
- « Quelle est ma tendance de forme sur le dernier mois ? »
- « Montre-moi mes plus longues courses cette année. »

---

## Déconnexion

Tapez `/logout` dans le chat Telegram. Dravr dissociera Telegram de votre compte. Vos données et appartenances aux groupes ne seront pas affectées.

---

## FAQ

**Je ne trouve pas le bot Dravr sur Telegram. Que faire ?**
Demandez à votre entraîneur ou admin le nom d'utilisateur exact du bot Telegram Dravr.

**J'ai entré la mauvaise adresse courriel. Puis-je recommencer ?**
Tapez `cancel` puis recommencez le processus en envoyant n'importe quel message.

**Puis-je utiliser Dravr dans un groupe Telegram ?**
La liaison du compte doit se faire en message direct avec le bot. Une fois lié, votre entraîneur peut vous ajouter à un groupe Telegram où Dravr est également présent.

---

**Voir aussi :** [Dravr sur la messagerie](/fr/docs/messaging) · [Connecter à WhatsApp](/fr/docs/whatsapp) · [Connecter à Slack](/fr/docs/slack) · [Connecter à Discord](/fr/docs/discord)
