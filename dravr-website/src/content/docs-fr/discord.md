---
title: Connecter à Discord
description: Liez votre compte Dravr à Discord et commencez à poser des questions dans votre serveur.
order: 2
platform: discord
---

Discord est idéal pour les communautés sportives et les serveurs d'entraînement en groupe. Une fois que Dravr a été ajouté à un serveur, les membres peuvent lier leurs comptes et poser des questions directement.

## Avant de commencer

- Vous avez besoin d'un compte Dravr. Si vous n'en avez pas, demandez à votre entraîneur ou à l'admin.
- Dravr doit avoir été ajouté à votre serveur Discord par un admin. Si vous ne voyez pas `@Dravr` dans votre serveur, demandez à votre admin de l'ajouter.

## Comment se connecter

1. Dans Discord, ouvrez un message direct avec le bot Dravr. Trouvez-le en cliquant sur son nom dans la liste des membres du serveur, ou en mentionnant `@Dravr` dans un canal puis en cliquant sur son profil.
2. Envoyez n'importe quel message (par exemple, « Bonjour ») pour démarrer la conversation.
3. Dravr répond : *« Bonjour ! Pour lier votre compte Dravr, veuillez entrer votre adresse courriel. »*
4. Entrez l'adresse courriel utilisée pour vous connecter à Dravr.
5. Dravr répond : *« J'ai envoyé un code à 6 chiffres à j***@votredomaine.com »*
6. Vérifiez votre courriel et entrez le code à 6 chiffres dans le message Discord.
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
| `/logout` | Dissocier Discord de votre compte Dravr |

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
4. Suivez ensuite les étapes de connexion ci-dessus pour lier Discord et discuter avec Dravr.

---

## Ce que vous pouvez demander à Dravr

- « Comment était ma charge d'entraînement cette semaine ? »
- « Suis-je assez récupéré pour une séance intense aujourd'hui ? »
- « Quelle est ma tendance de forme sur le dernier mois ? »
- « Montre-moi mes plus longues courses cette année. »

---

## Déconnexion

Tapez `/logout` dans le message direct avec Dravr. Dravr dissociera Discord de votre compte. Vos données et appartenances aux groupes ne seront pas affectées.

---

## FAQ

**Je ne vois pas Dravr dans mon serveur Discord. Que faire ?**
L'admin de votre serveur doit d'abord ajouter Dravr. Demandez-lui d'inviter le bot via le panneau d'administration Dravr.

**J'ai entré la mauvaise adresse courriel. Puis-je recommencer ?**
Tapez `cancel` puis recommencez le processus en envoyant n'importe quel message.

**Puis-je poser des questions à Dravr dans un canal de serveur plutôt qu'en message privé ?**
La liaison du compte doit se faire en message direct avec le bot. Une fois lié, vous pourrez peut-être interagir avec Dravr dans les canaux du serveur selon la configuration de votre admin.

---

**Voir aussi :** [Dravr sur la messagerie](/fr/docs/messaging) · [Connecter à Telegram](/fr/docs/telegram) · [Connecter à WhatsApp](/fr/docs/whatsapp) · [Connecter à Slack](/fr/docs/slack)
