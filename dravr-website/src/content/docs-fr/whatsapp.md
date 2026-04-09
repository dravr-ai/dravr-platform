---
title: Connecter à WhatsApp
description: Liez votre compte Dravr à WhatsApp et posez des questions comme vous communiquez déjà.
order: 5
platform: whatsapp
---

WhatsApp est un bon choix si vous et votre équipe communiquez déjà là-bas. Dravr apparaît comme un contact dans votre WhatsApp — envoyez-lui un message comme vous le feriez avec une personne.

## Avant de commencer

- Vous avez besoin d'un compte Dravr. Si vous n'en avez pas, demandez à votre entraîneur ou à l'admin.
- Votre entraîneur ou admin doit vous fournir le numéro de téléphone du bot WhatsApp Dravr.

## Comment se connecter

1. Ajoutez le numéro WhatsApp Dravr (fourni par votre entraîneur ou admin) à vos contacts.
2. Ouvrez une conversation et envoyez n'importe quel message (par exemple, « Bonjour ») pour démarrer.
3. Dravr répond : *« Bonjour ! Pour lier votre compte Dravr, veuillez entrer votre adresse courriel. »*
4. Entrez l'adresse courriel utilisée pour vous connecter à Dravr.
5. Dravr répond : *« J'ai envoyé un code à 6 chiffres à j***@votredomaine.com »*
6. Vérifiez votre courriel et entrez le code à 6 chiffres dans le chat WhatsApp.
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
| `/logout` | Dissocier WhatsApp de votre compte Dravr |

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
4. Suivez ensuite les étapes de connexion ci-dessus pour lier WhatsApp et discuter avec Dravr.

---

## Ce que vous pouvez demander à Dravr

- « Comment était ma charge d'entraînement cette semaine ? »
- « Suis-je assez récupéré pour une séance intense aujourd'hui ? »
- « Quelle est ma tendance de forme sur le dernier mois ? »
- « Montre-moi mes plus longues courses cette année. »

---

## Déconnexion

Tapez `/logout` dans le chat WhatsApp. Dravr dissociera WhatsApp de votre compte. Vos données et appartenances aux groupes ne seront pas affectées.

---

## FAQ

**Je ne reçois pas de réponse du bot WhatsApp. Que faire ?**
Assurez-vous d'avoir le bon numéro. Demandez à votre entraîneur ou admin de confirmer le numéro WhatsApp Dravr.

**J'ai entré la mauvaise adresse courriel. Puis-je recommencer ?**
Tapez `cancel` puis recommencez le processus en envoyant n'importe quel message.

**WhatsApp m'indique que le message n'a pas été livré. Que faire ?**
Le numéro WhatsApp Dravr doit d'abord être enregistré comme contact sur votre téléphone. Ajoutez-le à vos contacts et réessayez.

---

**Voir aussi :** [Dravr sur la messagerie](/fr/docs/messaging) · [Connecter à Telegram](/fr/docs/telegram) · [Connecter à Slack](/fr/docs/slack) · [Connecter à Discord](/fr/docs/discord)
