// ABOUTME: i18n translation strings for all UI copy across the Dravr website
// ABOUTME: Supports English (default) and French — add new keys here first, then translate

export const languages = {
  en: 'EN',
  fr: 'FR',
} as const;

export const defaultLang = 'en';

export const ui = {
  en: {
    // Nav
    'nav.about': 'About',
    'nav.docs': 'Docs',
    'nav.join': 'Join the Waitlist',
    'nav.mobile.open': 'Open menu',

    // Home
    'home.tagline': 'Navigate your health data.',
    'home.hero': 'Your health and fitness data, finally unified.',
    'home.body':
      'Dravr is an AI layer that connects your fitness, wellness, and health data — so you and your coach always have the full picture.',
    'home.email.placeholder': 'Enter your email',
    'home.cta': 'Join the Waitlist',
    'home.img.alt': 'Historical draveur on a river at sunset in the Quebec boreal forest',
    'home.status.label': 'Project Status: Alpha',
    'home.status.phase': 'Phase 01',
    'home.status.title': 'Early Access',
    'home.status.desc':
      'Building the orchestration layer for endurance athletes and coaches. Connecting Strava, Garmin, Whoop, and more.',
    'home.success': "You're on the list. We'll reach out soon.",
    'home.error': 'Something went wrong. Try again.',

    // Footer (home inline)
    'footer.privacy': 'Privacy Policy',
    'footer.terms': 'Terms of Service',
    'footer.docs': 'Documentation',
    'footer.rights': '© 2026 DRAVR. All rights reserved.',
    // Footer component
    'footer.alpha': 'Alpha Docs',
    'footer.short.rights': '© 2026 Dravr. All rights reserved.',

    // Docs sidebar & layout
    'docs.sidebar.alpha': 'Alpha Guides',
    'docs.sidebar.guides': 'Guides',
    'docs.sidebar.boreal.label': 'The current',
    'docs.sidebar.boreal.quote':
      '"The draveurs read the river\'s veins to guide their logs. We read your data to guide yours."',
    'docs.sidebar.signout': 'Sign out',
    'docs.sidebar.toggle': 'Toggle navigation',
    'docs.breadcrumb.guides': 'Guides',

    // Docs hub
    'docs.welcome': "Here's everything you need to get started.",
    'docs.welcome.name': 'Welcome, ',
    'docs.meta.description': 'Alpha user documentation for Dravr.',

    // Login page
    'login.title': 'Early Access',
    'login.subtitle': 'Dravr — Alpha Program',
    'login.card.title': 'Sign in with email',
    'login.card.desc': 'Enter your email and we\'ll send you a sign-in link.',
    'login.waitlist':
      'Your account is on the waitlist but not yet approved. We\'ll reach out when access opens.',
    'login.email.label': 'Your email',
    'login.email.placeholder': 'your@email.com',
    'login.submit': 'Continue',
    'login.request': 'Request Access',
    'login.back': '← Back to Dravr',
    'login.footer.node': '46.3482° N, 72.5428° W — Rivière Saint-Maurice',
    'login.footer.rights': '© 2026 DRAVR. ALL RIGHTS RESERVED.',
    'login.sending': 'Sending link…',
    'login.error.send': 'Could not send the link. Try again in a moment.',
    'login.check.email': 'Check your email — the magic link expires in 10 minutes.',
    'login.sent': 'Link sent ✓',
    'login.img.alt': 'Boreal River Background',
    'login.or': 'or',
    'login.google': 'Continue with Google',

    // Docs slug page nav
    'docs.nav.prev': 'Previous',
    'docs.nav.next': 'Next',
  },

  fr: {
    // Nav
    'nav.about': 'À propos',
    'nav.docs': 'Docs',
    'nav.join': 'Rejoindre la liste d\'attente',
    'nav.mobile.open': 'Ouvrir le menu',

    // Home
    'home.tagline': 'Naviguez vos données de santé.',
    'home.hero': 'Vos données de sport et santé, enfin unifiées.',
    'home.body':
      'Dravr est une couche IA qui connecte vos données de forme physique, de bien-être et de santé — pour que vous et votre entraîneur ayez toujours le portrait complet.',
    'home.email.placeholder': 'Votre adresse courriel',
    'home.cta': 'Rejoindre la liste d\'attente',
    'home.img.alt': 'Draveur historique sur une rivière au coucher de soleil dans la forêt boréale québécoise',
    'home.status.label': 'Statut du projet : Alpha',
    'home.status.phase': 'Phase 01',
    'home.status.title': 'Accès anticipé',
    'home.status.desc':
      'Construction de la couche d\'orchestration pour les athlètes d\'endurance et les entraîneurs. Connexion à Strava, Garmin, Whoop, et plus.',
    'home.success': 'Vous êtes sur la liste. On vous contactera bientôt.',
    'home.error': 'Une erreur s\'est produite. Réessayez.',

    // Footer (home inline)
    'footer.privacy': 'Politique de confidentialité',
    'footer.terms': 'Conditions d\'utilisation',
    'footer.docs': 'Documentation',
    'footer.rights': '© 2026 DRAVR. Tous droits réservés.',
    // Footer component
    'footer.alpha': 'Docs Alpha',
    'footer.short.rights': '© 2026 Dravr. Tous droits réservés.',

    // Docs sidebar & layout
    'docs.sidebar.alpha': 'Guides Alpha',
    'docs.sidebar.guides': 'Guides',
    'docs.sidebar.boreal.label': 'Le courant',
    'docs.sidebar.boreal.quote':
      '« Les draveurs lisaient les veines de la rivière pour guider leurs billots. Nous lisons vos données pour guider les vôtres. »',
    'docs.sidebar.signout': 'Se déconnecter',
    'docs.sidebar.toggle': 'Basculer la navigation',
    'docs.breadcrumb.guides': 'Guides',

    // Docs hub
    'docs.welcome': 'Voici tout ce qu\'il vous faut pour commencer.',
    'docs.welcome.name': 'Bienvenue, ',
    'docs.meta.description': 'Documentation alpha pour les utilisateurs Dravr.',

    // Login page
    'login.title': 'Accès anticipé',
    'login.subtitle': 'Dravr — Programme Alpha',
    'login.card.title': 'Se connecter par courriel',
    'login.card.desc': 'Entrez votre courriel et nous vous enverrons un lien de connexion.',
    'login.waitlist':
      'Votre compte est sur la liste d\'attente mais n\'a pas encore été approuvé. Nous vous contacterons à l\'ouverture des accès.',
    'login.email.label': 'Votre courriel',
    'login.email.placeholder': 'votre@courriel.com',
    'login.submit': 'Continuer',
    'login.request': 'Demander l\'accès',
    'login.back': '← Retour à Dravr',
    'login.footer.node': '46.3482° N, 72.5428° O — Rivière Saint-Maurice',
    'login.footer.rights': '© 2026 DRAVR. TOUS DROITS RÉSERVÉS.',
    'login.sending': 'Envoi du lien…',
    'login.error.send': 'Impossible d\'envoyer le lien. Réessayez dans un moment.',
    'login.check.email': 'Vérifiez votre courriel — le lien magique expire dans 10 minutes.',
    'login.sent': 'Lien envoyé ✓',
    'login.img.alt': 'Arrière-plan de rivière boréale',
    'login.or': 'ou',
    'login.google': 'Continuer avec Google',

    // Docs slug page nav
    'docs.nav.prev': 'Précédent',
    'docs.nav.next': 'Suivant',
  },
} as const;
