// The French catalogue (DESIGN.md §8 "Français"): tutoiement, sentence case, Discord's French
// words, and French typography. Typed against the English one, so every key must be here.

import { MAXIMUM_COUNTED_UNREAD_MENTIONS } from "../lib/protocol";
import type { Catalogue } from "./en";
import { pluralChooser } from "./plural";

/** Before « ? », « ! » and « ; ». */
const NARROW_NO_BREAK_SPACE = " ";
/** Before « : », inside « guillemets » and between a number and its unit. */
const NO_BREAK_SPACE = " ";

const plural = pluralChooser("fr");
const count = (value: number) => value.toLocaleString("fr");
const FRENCH_SIZE_UNITS = { KB: "Ko", MB: "Mo", GB: "Go" } as const;

export const fr: Catalogue = {
  common: {
    cancel: "Annuler",
    close: "Fermer",
    tryAgain: "Réessayer",
    save: "Enregistrer",
    saving: "Enregistrement…",
    edit: "Modifier",
    delete: "Supprimer",
    online: "En ligne",
    offline: "Hors ligne",
    withUnread: (name) => `${name}, non lu`,
    unreadSuffix: ", non lu",
    withMentions: (name, mentions) =>
      mentions >= MAXIMUM_COUNTED_UNREAD_MENTIONS
        ? `${name}, ${count(MAXIMUM_COUNTED_UNREAD_MENTIONS - 1)}+ mentions`
        : `${name}, ${plural(mentions, { one: `${count(mentions)} mention`, other: `${count(mentions)} mentions` })}`,
    readOnly: "lecture seule",
    someone: "Quelqu’un",
  },

  errors: {
    withReason: (summary, reason) => `${summary}${NO_BREAK_SPACE}: ${reason}`,
    unknownReason: "un problème est survenu. Réessaie.",
    reasons: {
      NETWORK: "l’instance est injoignable. Réessaie.",
      INVALID_CREDENTIALS: "l’identifiant ou le mot de passe est incorrect.",
      USERNAME_TAKEN: "ce nom d’utilisateur est déjà pris.",
      EMAIL_TAKEN: "un compte utilise déjà cette adresse e-mail.",
      REGISTRATION_CLOSED: "cette instance n’accepte pas de nouveaux comptes.",
      VALIDATION_FAILED: "l’instance a refusé ce qui a été saisi.",
      RATE_LIMITED: "trop de tentatives. Patiente un instant, puis réessaie.",
      SERVER_BUSY: "l’instance est surchargée. Réessaie dans un instant.",
      MISSING_PERMISSIONS: "tu n’as pas la permission de faire ça ici.",
      NOT_FOUND: "ça n’existe plus, ou tu ne peux pas le voir.",
      INVALID_INVITE: "cette invitation est inconnue, expirée ou épuisée.",
      BANNED: "un bannissement t’empêche d’accéder à ce serveur.",
      OWNER_CANNOT_LEAVE: "tu ne peux pas quitter un serveur qui t’appartient.",
      LIMIT_REACHED: "une limite de cette instance est atteinte.",
      FILE_TOO_LARGE: "le fichier dépasse la taille acceptée par cette instance.",
    },
  },

  app: {
    unreachableTitle: "Impossible de joindre cette instance pour le moment.",
    unreachableBody: "Vérifie ta connexion, puis réessaie.",
  },

  connection: {
    connecting: "Connexion…",
    reconnecting: "Connexion perdue. Reconnexion…",
  },

  time: {
    todayAt: (time) => `Aujourd’hui à ${time}`,
    yesterdayAt: (time) => `Hier à ${time}`,
  },

  fileSize: {
    bytes: (bytes) => plural(bytes, { one: `${count(bytes)}${NO_BREAK_SPACE}octet`, other: `${count(bytes)}${NO_BREAK_SPACE}octets` }),
    inUnit: (value, unit) => `${value}${NO_BREAK_SPACE}${FRENCH_SIZE_UNITS[unit]}`,
  },

  typing: {
    several: "Plusieurs personnes écrivent…",
    isTyping: " écrit…",
    and: " et ",
    areTyping: " écrivent…",
    listSeparator: ", ",
    andOthersAreTyping: (others) =>
      ` et ${count(others)} ${plural(others, { one: "autre", other: "autres" })} écrivent…`,
  },

  signIn: {
    signInTitle: (instance) => `Se connecter à ${instance}`,
    registerTitle: (instance) => `Créer un compte sur ${instance}`,
    signInSubtitle: "Ton compte est hébergé sur cette instance.",
    registerSubtitle: "Ton compte sera hébergé sur cette instance.",
    email: "E-mail",
    username: "Nom d’utilisateur",
    usernameHelper: `De 2 à 32 caractères${NO_BREAK_SPACE}: lettres minuscules, chiffres, points et tirets bas.`,
    password: "Mot de passe",
    passwordLengthHelper: (minimum) => `Au moins ${count(minimum)} caractères.`,
    login: "Nom d’utilisateur ou e-mail",
    signIn: "Se connecter",
    signingIn: "Connexion…",
    createAccount: "Créer le compte",
    creatingAccount: "Création du compte…",
    hasAccount: `Tu as déjà un compte${NARROW_NO_BREAK_SPACE}?`,
    hasNoAccount: `Pas encore de compte${NARROW_NO_BREAK_SPACE}?`,
    createAnAccount: "Créer un compte",
    signInFailed: "Impossible de te connecter",
    registerFailed: "Impossible de créer ton compte",
    showPassword: "Afficher le mot de passe",
  },

  rail: {
    servers: "Serveurs",
    directMessages: "Messages privés",
    addServer: "Ajouter un serveur",
  },

  shell: {
    serversAndChannels: "Serveurs et salons",
    membersOf: (server) => `Membres de ${server}`,
  },

  sidebar: {
    directMessages: "Messages privés",
    serverMenu: "Menu du serveur",
    invitePeople: "Inviter des gens",
    createChannel: "Créer un salon",
    serverSettings: "Paramètres du serveur",
    leaveServer: "Quitter le serveur",
    channelsIn: (server) => `Salons de ${server}`,
    noConversations:
      "Aucune conversation pour l’instant. Ouvre le profil de quelqu’un dans la liste des membres d’un serveur et choisis Envoyer un message.",
    justYou: "Rien que toi",
    noChannelsYet: "Aucun salon pour l’instant. Crée le premier pour lancer la conversation.",
    createAChannel: "Créer un salon",
    noVisibleChannelsYet: "Aucun salon visible pour l’instant. Demande l’accès à la modération.",
    createChannelIn: (category) => `Créer un salon dans ${category}`,
    editChannel: (channel) => `Modifier le salon ${channel}`,
  },

  userCard: {
    muteMicrophone: "Couper le micro",
    unmuteMicrophone: "Réactiver le micro",
    deafen: "Mettre en sourdine",
    undeafen: "Désactiver la sourdine",
    userSettings: "Paramètres utilisateur",
  },

  chat: {
    chat: "Discussion",
    openNavigation: "Ouvrir les serveurs et les salons",
    goToLabel: "Aller à un salon, un serveur ou une personne",
    goTo: "Aller à…",
    memberList: "Liste des membres",
    channelSettings: "Paramètres du salon",
    markAsRead: "Marquer comme lu",
    newMessagesSince: (newCount, isPartial, since) =>
      `${count(newCount)}${isPartial ? "+" : ""} ${plural(isPartial ? 2 : newCount, { one: "nouveau message", other: "nouveaux messages" })} depuis ${since}`,
    composerInChannel: (channel) => `Écrire dans #${channel}`,
    composerToPerson: (person) => `Écrire à @${person}`,
    welcomeToChannel: (channel) => `Bienvenue dans #${channel}`,
    channelStart: (channel) => `C’est le début du salon #${channel}.`,
    conversationStart: (person) => `C’est le début de ton historique de messages privés avec ${person}.`,
    replyingTo: "En réponse à",
    replyHint: (person) => `En réponse à ${person}. Échap annule la réponse.`,
    cancelReply: "Annuler la réponse",
    attachFiles: "Joindre des fichiers",
    sendMessage: "Envoyer le message",
    cannotSend: "Tu n’as pas la permission d’envoyer des messages dans ce salon.",
    dropToAttach: (target) => `Dépose pour joindre à ${target}`,
    dropHint: "Tu pourras ajouter un message avant l’envoi.",
    tooManyAttachments: (maximum) => `Tu peux joindre jusqu’à ${count(maximum)} fichiers par message.`,
    uploadsRunning: "Tes fichiers sont encore en cours d’envoi. Renvoie le message quand ils sont prêts.",
    uploaded: (fileName) => `${fileName} envoyé.`,
    fileTooLarge: (fileName, size, instance) => `${fileName} fait ${size}, plus que ce qu’accepte ${instance}.`,
    uploadFailed: (fileName) => `Impossible d’envoyer ${fileName}`,
    editFailed: "Impossible de modifier le message",
    deleteFailed: "Impossible de supprimer le message",
    copyFailed: "Impossible de copier le texte. Sélectionne-le et copie-le à la main.",
    loadingWorkspace: "Chargement de tes serveurs et conversations…",
    noVisibleChannelsTitle: "Aucun salon visible",
    noVisibleChannelsBody: (server) => `Les salons que tu peux lire dans ${server} s’afficheront ici.`,
    noServersTitle: "Aucun serveur pour l’instant",
    noServersBody: "Crée un serveur pour ton groupe, ou rejoins-en un avec un code d’invitation.",
    addServer: "Ajouter un serveur",
    noConversationsTitle: "Aucune conversation pour l’instant",
    noConversationsBody: `Lances-en une avec quelqu’un de tes serveurs${NO_BREAK_SPACE}: ouvre son profil dans la liste des membres et choisis Envoyer un message.`,
  },

  messages: {
    loading: "Chargement des messages",
    startOf: (channel) => `Début de ${channel}`,
    loadFailed: (channel) => `Impossible de charger les messages de ${channel}.`,
    loadOlderFailed: "Impossible de charger les messages plus anciens.",
    messagesIn: (channel) => `Messages dans ${channel}`,
    newSince: (time) => `Nouveaux messages depuis ${time}`,
    newPill: "Nouveau",
    viewingOlder: "Tu consultes des messages plus anciens",
    jumpToPresent: "Revenir au présent",
    originalUnavailable: "Le message d’origine a été supprimé ou n’est pas chargé",
    authorSaid: (author, text) => `${author}${NO_BREAK_SPACE}: ${text}`,
    attachmentCount: (attachments) =>
      plural(attachments, { one: `${count(attachments)} pièce jointe`, other: `${count(attachments)} pièces jointes` }),
    edited: "(modifié)",
    editedAt: (timestamp) => `Modifié le ${timestamp}`,
    mentionsYou: "Te mentionne.",
    spoilerHidden: "Spoiler, sélectionne pour l’afficher",
    spoilerRevealedPrefix: `Spoiler${NO_BREAK_SPACE}: `,
    editMessage: "Modifier le message",
    escTo: "Échap pour",
    cancelLowercase: "annuler",
    enterTo: "· Entrée pour",
    saveLowercase: "enregistrer",
    cancelEditing: "Annuler la modification",
    saveChanges: "Enregistrer les modifications",
    sending: "Envoi…",
    notSent: "Non envoyé.",
    retry: "Réessayer",
    retryLabel: "Réessayer d’envoyer ce message",
    deleteUnsentLabel: "Supprimer ce message non envoyé",
    actions: "Actions du message",
    reply: "Répondre",
    copyText: "Copier le texte",
    more: "Plus",
    download: (fileName) => `Télécharger ${fileName}`,
  },

  reactions: {
    reactions: "Réactions",
    addReaction: "Ajouter une réaction",
    reactWith: (emoji) => `Réagir avec ${emoji}`,
    chip: (emoji, reactions) =>
      `${emoji}, ${count(reactions)}${NO_BREAK_SPACE}${plural(reactions, { one: "réaction", other: "réactions" })}`,
    reactedWith: (includesYou, names, others, emoji) => {
      const people = [...(includesYou ? ["Toi"] : []), ...names];
      if (others > 0) {
        people.push(`${count(others)}${NO_BREAK_SPACE}${plural(others, { one: "autre", other: "autres" })}`);
      }
      if (people.length === 1) {
        return includesYou
          ? `Tu as réagi avec${NO_BREAK_SPACE}${emoji}`
          : `${people[0]} a réagi avec${NO_BREAK_SPACE}${emoji}`;
      }
      const list = `${people.slice(0, -1).join(", ")} et ${people.at(-1)}`;
      return `${list} ${includesYou ? "avez" : "ont"} réagi avec${NO_BREAK_SPACE}${emoji}`;
    },
    loadingWho: "Chargement des réactions…",
    whoFailed: "Impossible de savoir qui a réagi.",
    failed: "Impossible de réagir",
    removeShort: "Retirer",
    remove: (person) => `Retirer la réaction de ${person}`,
    removeFailed: "Impossible de retirer la réaction",
    removeAll: "Retirer toutes les réactions",
    removeAllTitle: `Retirer toutes les réactions${NARROW_NO_BREAK_SPACE}?`,
    removeAllSubtitle: "Toutes les réactions de ce message seront retirées pour tout le monde.",
    removeAllSubmit: "Tout retirer",
    removeAllFailed: "Impossible de retirer les réactions",
    noReactions: "Plus aucune réaction.",
  },

  emojiPicker: {
    label: "Sélecteur d’emoji",
    search: "Rechercher un emoji",
    placeholder: "Trouve l’emoji parfait",
    recent: "Utilisés récemment",
    results: "Résultats de la recherche",
    noResults: (query) => `Aucun emoji pour «${NO_BREAK_SPACE}${query}${NO_BREAK_SPACE}».`,
    help: "Flèches pour te déplacer, Entrée pour choisir, Échap pour fermer.",
    categories: {
      people: "Personnes",
      nature: "Nature",
      food: "Nourriture et boissons",
      activities: "Activités",
      travel: "Voyages et lieux",
      objects: "Objets",
      symbols: "Symboles",
      flags: "Drapeaux",
    },
  },

  pins: {
    pinnedMessages: "Messages épinglés",
    pin: "Épingler le message",
    unpin: "Désépingler le message",
    pinned: "Épinglé",
    pinTitle: `Épingler ce message${NARROW_NO_BREAK_SPACE}?`,
    pinSubtitle: (channel) => `Il apparaîtra dans les messages épinglés de ${channel}.`,
    pinSubmit: "Épingler",
    unpinTitle: `Désépingler ce message${NARROW_NO_BREAK_SPACE}?`,
    unpinSubtitle: "Il n’apparaîtra plus dans les messages épinglés.",
    unpinSubmit: "Désépingler",
    pinFailed: "Impossible d’épingler le message",
    unpinFailed: "Impossible de désépingler le message",
    loading: "Chargement des messages épinglés…",
    loadFailed: "Impossible de charger les messages épinglés.",
    emptyTitle: "Aucun message épinglé pour l’instant",
    emptyBody: "Épingle un message depuis son menu Plus pour le garder ici, à la vue de tous.",
    jump: "Voir",
    jumpTo: (author) => `Aller au message de ${author}`,
  },

  attachmentTray: {
    filesToSend: "Fichiers à envoyer",
    uploading: "Envoi…",
    remove: (fileName) => `Retirer ${fileName}`,
  },

  deleteMessage: {
    title: `Supprimer ce message${NARROW_NO_BREAK_SPACE}?`,
    subtitle: "Il sera supprimé pour tout le monde.",
    submit: "Supprimer le message",
    tip: `Astuce${NO_BREAK_SPACE}: maintiens Maj en supprimant pour passer cette étape la prochaine fois.`,
  },

  addServer: {
    joinTitle: "Rejoindre un serveur",
    createTitle: "Créer un serveur",
    joinSubtitle: "Saisis une invitation pour rejoindre un serveur qui existe déjà.",
    createSubtitle: "Donne un nom à ton serveur. Tu pourras ensuite ajouter des salons et inviter des gens.",
    join: "Rejoindre",
    joining: "Ajout du serveur…",
    create: "Créer",
    creating: "Création…",
    inviteCode: "Code d’invitation",
    inviteCodeHelper: "Colle le code d’invitation qu’on t’a donné.",
    serverName: "Nom du serveur",
    hasNoInvite: `Pas d’invitation${NARROW_NO_BREAK_SPACE}?`,
    hasInvite: `Tu as déjà une invitation${NARROW_NO_BREAK_SPACE}?`,
    joinFailed: "Impossible de rejoindre le serveur",
    createFailed: "Impossible de créer le serveur",
  },

  createChannel: {
    title: "Créer un salon",
    inCategory: (category) => `dans ${category}`,
    submit: "Créer le salon",
    creating: "Création…",
    channelName: "Nom du salon",
    placeholder: "nouveau-salon",
    helper: "Les noms sont en minuscules, avec des tirets à la place des espaces.",
    failed: "Impossible de créer le salon",
  },

  invitePeople: {
    title: (server) => `Inviter des gens sur ${server}`,
    failed: "Impossible de créer une invitation",
    copyFailed: "Impossible de copier le code. Sélectionne-le et copie-le à la main.",
    codeLabel: "Envoie ce code d’invitation à quelqu’un",
    creating: "Création de l’invitation…",
    copy: "Copier",
    copied: "Copié",
    neverExpires: "Cette invitation n’expire jamais.",
    codeCopied: "Code d’invitation copié",
  },

  leaveServer: {
    title: (server) => `Quitter «${NO_BREAK_SPACE}${server}${NO_BREAK_SPACE}»`,
    subtitle: (server) =>
      `Tu veux vraiment quitter ${server}${NARROW_NO_BREAK_SPACE}? Tu ne pourras revenir qu’avec une nouvelle invitation.`,
    submit: "Quitter le serveur",
    leaving: "Départ du serveur…",
    failed: "Impossible de quitter le serveur",
  },

  members: {
    members: "Membres",
    profile: "Profil",
    profileOf: (person) => `Profil de ${person}`,
    backToMembers: "Retour aux membres",
    sendMessage: "Envoyer un message",
    opening: "Ouverture…",
    openConversationFailed: "Impossible d’ouvrir la conversation",
    loadFailed: "Impossible de charger les membres.",
    loading: "Chargement des membres…",
    groupLabel: (group, members) => `${group}, ${count(members)} ${plural(members, { one: "membre", other: "membres" })}`,
    memberLabel: (person, isOnline) => `${person}, ${isOnline ? "en ligne" : "hors ligne"}`,
  },

  quickSwitcher: {
    label: "Aller à",
    placeholder: `Où veux-tu aller${NARROW_NO_BREAK_SPACE}?`,
    results: "Résultats",
    channels: "Salons",
    people: "Personnes",
    servers: "Serveurs",
    directMessage: "Message privé",
    server: "Serveur",
    noResults: (query) =>
      `Aucun résultat pour «${NO_BREAK_SPACE}${query}${NO_BREAK_SPACE}». Vérifie l’orthographe ou cherche un autre serveur.`,
    resultCount: (results) => `${count(results)} ${plural(results, { one: "résultat", other: "résultats" })}`,
    help: `Flèches pour choisir, Entrée pour ouvrir, Échap pour fermer. Préfixes${NO_BREAK_SPACE}: @${NO_BREAK_SPACE}personnes, #${NO_BREAK_SPACE}salons, *${NO_BREAK_SPACE}serveurs.`,
    openConversationFailed: (person) => `Impossible d’ouvrir la conversation avec ${person}`,
  },

  serverSettings: {
    serverSettings: "Paramètres du serveur",
    moderation: "Modération",
    sections: {
      overview: "Vue d’ensemble",
      roles: "Rôles",
      invites: "Invitations",
      members: "Membres",
      bans: "Bannissements",
      "moderation-log": "Journal de modération",
    },
    deleteServer: "Supprimer le serveur",
    backToServerSettings: "Retour aux paramètres du serveur",
    loading: "Chargement…",

    unsaved: {
      careful: `Attention${NO_BREAK_SPACE}: tu as des modifications non enregistrées.`,
      saveOrReset: "Enregistre ou annule tes modifications avant de partir.",
      reset: "Réinitialiser",
      saveChanges: "Enregistrer les modifications",
      saving: "Enregistrement…",
      saved: "Modifications enregistrées",
    },

    overview: {
      serverName: "Nom du serveur",
      serverNameHelper: "Jusqu’à 100 caractères. Tout le monde sur le serveur le voit.",
      nameRequired: "Donne un nom au serveur.",
      saveFailed: "Impossible d’enregistrer le nom du serveur",
    },

    deleteServerDialog: {
      title: (server) => `Supprimer «${NO_BREAK_SPACE}${server}${NO_BREAK_SPACE}»`,
      body: (server) =>
        `${server} sera supprimé pour tout le monde, avec ses salons, ses messages, ses rôles et ses invitations. C’est définitif.`,
      confirmLabel: "Saisis le nom du serveur pour confirmer",
      submit: "Supprimer le serveur",
      deleting: "Suppression…",
      failed: "Impossible de supprimer le serveur",
    },

    roles: {
      createRole: "Créer un rôle",
      creating: "Création…",
      newRoleName: "nouveau rôle",
      empty: "Les rôles servent à regrouper des gens et à leur donner des permissions.",
      roleList: "Rôles, du plus haut au plus bas",
      memberCount: (members) =>
        plural(members, { one: `${count(members)} membre`, other: `${count(members)} membres` }),
      locked: "Tu ne peux modifier que les rôles placés sous ton rôle le plus haut.",
      lockedRole: (role) => `${role}, verrouillé`,
      display: "Affichage",
      permissions: "Permissions",
      roleName: "Nom du rôle",
      everyoneCannotBeRenamed: "@everyone ne peut pas être renommé.",
      everyoneNote: "Tout le monde sur le serveur a @everyone. Ses permissions s’appliquent à tous les membres.",
      color: "Couleur du rôle",
      noColor: "Aucune couleur",
      colorOption: (hex) => `Couleur ${hex}`,
      colorPreview: "Les noms avec ce rôle",
      moveUp: "Monter",
      moveDown: "Descendre",
      moveHint: "Ctrl+Maj+Haut et Ctrl+Maj+Bas déplacent aussi le rôle choisi.",
      deleteRole: "Supprimer le rôle",
      deleteTitle: (role) => `Supprimer le rôle «${NO_BREAK_SPACE}${role}${NO_BREAK_SPACE}»${NARROW_NO_BREAK_SPACE}?`,
      deleteBody: (role) =>
        `Les membres qui ont ${role} le perdent, avec les permissions qu’il donne. C’est définitif.`,
      deleting: "Suppression…",
      nameRequired: "Donne un nom au rôle.",
      sensitive: "Sensible",
      permissionLocked: "Tu ne peux donner ou retirer que les permissions que tu as.",
      createFailed: "Impossible de créer le rôle",
      saveFailed: "Impossible d’enregistrer le rôle",
      deleteFailed: "Impossible de supprimer le rôle",
      moveFailed: "Impossible de déplacer le rôle",
      groups: {
        general: "Général",
        membership: "Adhésion",
        textChannels: "Salons textuels",
        moderation: "Modération",
        advanced: "Avancé",
      },
    },

    permissions: {
      viewChannels: { label: "Voir les salons", description: "Voir les salons textuels et lire ce qui y est publié." },
      manageChannels: { label: "Gérer les salons", description: "Créer, modifier et supprimer des salons." },
      manageRoles: {
        label: "Gérer les rôles",
        description: "Créer des rôles, et modifier ou supprimer ceux placés sous son rôle le plus haut.",
      },
      manageServer: {
        label: "Gérer le serveur",
        description: "Renommer le serveur et gérer ses invitations.",
      },
      viewModerationLog: {
        label: "Voir le journal de modération",
        description: "Voir qui a changé quoi sur ce serveur.",
      },
      createInvite: { label: "Créer une invitation", description: "Inviter de nouvelles personnes sur ce serveur." },
      sendMessages: { label: "Envoyer des messages", description: "Écrire dans les salons textuels." },
      readMessageHistory: {
        label: "Voir les anciens messages",
        description: "Lire les messages envoyés avant d’avoir ouvert le salon.",
      },
      attachFiles: { label: "Joindre des fichiers", description: "Ajouter des fichiers et des images à ses messages." },
      addReactions: { label: "Ajouter des réactions", description: "Réagir aux messages avec un nouvel emoji." },
      mentionEveryone: {
        label: "Mentionner @everyone",
        description: "Prévenir tout le monde dans un salon en une fois.",
      },
      manageMessages: {
        label: "Gérer les messages",
        description: "Supprimer les messages envoyés par d’autres membres.",
      },
      kickMembers: {
        label: "Expulser des membres",
        description: "Retirer des gens du serveur, qui peuvent revenir avec une nouvelle invitation.",
      },
      banMembers: {
        label: "Bannir des membres",
        description: "Retirer des gens du serveur et les empêcher de revenir.",
      },
      administrator: {
        label: "Administrateur",
        description:
          "Toutes les permissions dans tous les salons, quels que soient leurs réglages. À donner avec prudence.",
      },
    },

    members: {
      search: "Rechercher des membres",
      searchHelper: "Par nom affiché ou nom d’utilisateur.",
      noMatch: (query) =>
        `Aucun membre ne correspond à «${NO_BREAK_SPACE}${query}${NO_BREAK_SPACE}». Vérifie l’orthographe.`,
      loadFailed: "Impossible de charger les membres.",
      owner: "Propriétaire",
      memberRoles: (person) => `Rôles de ${person}`,
      manage: "Gérer",
      manageLabel: (person) => `Gérer ${person}`,
      addRole: "Ajouter un rôle…",
      addRoleTo: (person) => `Ajouter un rôle à ${person}`,
      removeRole: (role, person) => `Retirer ${role} à ${person}`,
      kick: "Expulser",
      ban: "Bannir",
      kickLabel: (person) => `Expulser ${person}`,
      banLabel: (person) => `Bannir ${person}`,
      kickTitle: (person, server) => `Expulser ${person} de ${server}${NARROW_NO_BREAK_SPACE}?`,
      kickBody: (person) =>
        `${person} ne fera plus partie du serveur. Une nouvelle invitation lui permettra de revenir.`,
      kicking: "Expulsion…",
      banTitle: (person, server) => `Bannir ${person} de ${server}${NARROW_NO_BREAK_SPACE}?`,
      banBody: (person) =>
        `${person} ne fera plus partie du serveur et ne pourra pas revenir, même avec une invitation.`,
      banReason: "Raison (facultatif, visible par la modération)",
      banning: "Bannissement…",
      kickFailed: (person) => `Impossible d’expulser ${person}`,
      banFailed: (person) => `Impossible de bannir ${person}`,
      roleFailed: "Impossible de modifier les rôles",
      kicked: (person) => `${person} a été expulsé du serveur`,
      banned: (person) => `${person} a été banni du serveur`,
    },

    bans: {
      empty: (server) => `Personne n’est banni de ${server}.`,
      loadFailed: "Impossible de charger les bannissements.",
      noReason: "Aucune raison donnée.",
      bannedOn: (date) => `Banni le ${date}`,
      revoke: "Révoquer le bannissement",
      revokeLabel: (person) => `Révoquer le bannissement de ${person}`,
      revoking: "Révocation…",
      revokeFailed: (person) => `Impossible de révoquer le bannissement de ${person}`,
      revoked: (person) => `${person} peut de nouveau rejoindre le serveur`,
    },

    invites: {
      empty: "Aucune invitation active.",
      loadFailed: "Impossible de charger les invitations.",
      createInvite: "Créer une invitation",
      creating: "Création…",
      createFailed: "Impossible de créer une invitation",
      created: (code) => `Invitation ${code} créée`,
      createdBy: (person) => `Créée par ${person}`,
      uses: (uses, maximum) =>
        maximum === null
          ? plural(uses, { one: `${count(uses)} utilisation`, other: `${count(uses)} utilisations` })
          : `${count(uses)} utilisations sur ${count(maximum)}`,
      expires: (date) => `Expire le ${date}`,
      neverExpires: "N’expire jamais",
      revoke: "Révoquer",
      revokeLabel: (code) => `Révoquer l’invitation ${code}`,
      revokeTitle: `Révoquer cette invitation${NARROW_NO_BREAK_SPACE}?`,
      revokeBody: "Le lien ne fonctionnera plus. Les personnes déjà arrivées restent sur le serveur.",
      revokeSubmit: "Révoquer l’invitation",
      revoking: "Révocation…",
      revokeFailed: "Impossible de révoquer l’invitation",
    },

    moderationLog: {
      empty: "Rien d’enregistré pour l’instant. Les changements de rôles, de salons, d’invitations et de membres s’afficheront ici.",
      loadFailed: "Impossible de charger le journal de modération.",
      loadOlder: "Charger les entrées plus anciennes",
      loadingOlder: "Chargement des entrées plus anciennes…",
      loadOlderFailed: "Impossible de charger les entrées plus anciennes.",
      reason: (reason) => `Raison${NO_BREAK_SPACE}: ${reason}`,
      deletedAccount: "Un compte supprimé",
      formerMember: "un ancien membre",
      deletedRole: "un rôle supprimé",
      deletedChannel: "un salon supprimé",
      channelCreate: (actor, channel) => `${actor} a créé #${channel}`,
      channelUpdate: (actor, channel) => `${actor} a modifié #${channel}`,
      channelDelete: (actor) => `${actor} a supprimé un salon`,
      overwriteSet: (actor, target, channel) => `${actor} a changé ce que ${target} peut faire dans #${channel}`,
      overwriteDelete: (actor, target, channel) => `${actor} a retiré les permissions de ${target} dans #${channel}`,
      roleCreate: (actor, role) => `${actor} a créé le rôle ${role}`,
      roleUpdate: (actor, role) => `${actor} a modifié le rôle ${role}`,
      roleDelete: (actor) => `${actor} a supprimé un rôle`,
      rolesReorder: (actor) => `${actor} a réordonné les rôles`,
      memberRoleAdd: (actor, role, member) => `${actor} a donné ${role} à ${member}`,
      memberRoleRemove: (actor, role, member) => `${actor} a retiré ${role} à ${member}`,
      memberKick: (actor, member) => `${actor} a expulsé ${member}`,
      memberBanAdd: (actor, member) => `${actor} a banni ${member}`,
      memberBanRemove: (actor, member) => `${actor} a révoqué le bannissement de ${member}`,
      messageDelete: (actor, author, channel) => `${actor} a supprimé un message de ${author} dans #${channel}`,
      messagePin: (actor, channel) => `${actor} a épinglé un message dans #${channel}`,
      messageUnpin: (actor, channel) => `${actor} a désépinglé un message dans #${channel}`,
      messageReactionRemove: (actor, channel) => `${actor} a retiré des réactions d’un message dans #${channel}`,
      guildRename: (actor, before, after) => `${actor} a renommé le serveur ${before} en ${after}`,
      guildUpdate: (actor) => `${actor} a modifié le serveur`,
      inviteCreate: (actor, code) => `${actor} a créé l’invitation ${code}`,
      inviteDelete: (actor, code) => `${actor} a révoqué l’invitation ${code}`,
      unknown: (actor) => `${actor} a fait un changement que cette version de Mellow ne sait pas encore afficher`,
      nameChange: (before, after) => `Nom${NO_BREAK_SPACE}: ${before}, devenu ${after}`,
      topicChange: (topic) => `Nouveau sujet${NO_BREAK_SPACE}: ${topic}`,
      topicCleared: "Sujet retiré",
      colorChange: "Couleur changée",
      permissionsAdded: (key, names) =>
        key === "deny"
          ? `Désormais refusé${NO_BREAK_SPACE}: ${names}`
          : key === "allow"
            ? `Désormais autorisé${NO_BREAK_SPACE}: ${names}`
            : `Ajouté${NO_BREAK_SPACE}: ${names}`,
      permissionsRemoved: (key, names) =>
        key === "deny"
          ? `Plus refusé${NO_BREAK_SPACE}: ${names}`
          : key === "allow"
            ? `Plus autorisé${NO_BREAK_SPACE}: ${names}`
            : `Retiré${NO_BREAK_SPACE}: ${names}`,
    },
  },

  channelSettings: {
    channelSettings: "Paramètres du salon",
    channelSettingsOf: (channel) => `Paramètres du salon, ${channel}`,
    sections: {
      overview: "Vue d’ensemble",
      permissions: "Permissions",
    },
    backToChannelSettings: "Retour aux paramètres du salon",

    overview: {
      channelName: "Nom du salon",
      channelNameHelper: "En minuscules, avec des tirets à la place des espaces. Jusqu’à 100 caractères.",
      topic: "Sujet",
      topicHelper: `Affiché en haut du salon. Jusqu’à 1${NARROW_NO_BREAK_SPACE}024 caractères.`,
      nameRequired: "Donne un nom au salon.",
      saveFailed: "Impossible d’enregistrer le salon",
    },

    permissions: {
      intro: (channel) =>
        `Choisis qui peut voir et utiliser #${channel}. «${NO_BREAK_SPACE}Héritée${NO_BREAK_SPACE}» reprend ce que leurs rôles permettent sur tout le serveur.`,
      targets: "Rôles et membres",
      add: "Ajouter un rôle ou un membre",
      addTitle: (channel) => `Ajouter à #${channel}`,
      addSubtitle: `Ses permissions ici sont d’abord héritées${NARROW_NO_BREAK_SPACE}; tu les changes une fois qu’il est dans la liste.`,
      search: "Rechercher un rôle ou un membre",
      roles: "Rôles",
      members: "Membres",
      noMatch: (query) =>
        `Aucun rôle ni membre ne correspond à «${NO_BREAK_SPACE}${query}${NO_BREAK_SPACE}». Vérifie l’orthographe.`,
      nothingToAdd: "Tous les rôles et membres que tu peux gérer sont déjà dans la liste.",
      loadingMembers: "Chargement des membres…",
      addFailed: "Impossible de l’ajouter au salon",
      remove: "Retirer",
      removeLabel: (name) => `Retirer ${name} des permissions de ce salon`,
      removing: "Retrait…",
      removeFailed: "Impossible de le retirer du salon",
      saveFailed: "Impossible d’enregistrer les permissions",
      lockedTarget: "Tu ne peux modifier que les rôles placés sous ton rôle le plus haut.",
      lockedTargetRow: (name) => `${name}, verrouillé`,
      lockedPermissions: "Tu ne peux modifier que les permissions que tu as dans ce salon.",
      everyoneNote:
        "@everyone s’applique à tout le monde sur le serveur. Les rôles et les membres de la liste passent avant.",
      states: { deny: "Refusée", inherit: "Héritée", allow: "Autorisée" },
      channelWords: {
        viewChannels: { label: "Voir le salon", description: "Voir ce salon et lire ce qui y est publié." },
        manageChannels: {
          label: "Gérer le salon",
          description: "Renommer ce salon et changer son sujet et son icône.",
        },
        manageRoles: { label: "Gérer les permissions", description: "Changer qui peut faire quoi dans ce salon." },
      },
    },
  },

  iconEditor: {
    formats: (maximumSize) =>
      `PNG, JPEG, GIF ou WebP, jusqu’à ${maximumSize}. Tu choisis le carré une fois l’image ouverte.`,
    server: {
      title: "Icône du serveur",
      emptyText: "Les initiales du serveur s’affichent tant que tu n’as pas ajouté d’icône.",
      note: "Affichée dans la liste des serveurs. Les GIF animés s’animent au survol.",
    },
    channel: {
      title: "Icône du salon",
      emptyText: "Le # s’affiche tant que tu n’as pas ajouté d’icône.",
      note: "Affichée en petit et sans animation, à la place du # devant le nom du salon.",
    },
    upload: "Envoyer une icône",
    uploading: "Envoi de l’icône…",
    remove: "Retirer l’icône",
    removing: "Retrait de l’icône…",
    saved: "Icône enregistrée.",
    removed: "Icône retirée.",
    changeFailed: "Impossible de changer l’icône",
    removeFailed: "Impossible de retirer l’icône",
    crop: {
      title: "Choisis le carré",
      subtitle:
        "Fais glisser l’image et zoome jusqu’à ce que le carré contienne ce que tu veux montrer. Le reste est coupé.",
      stage: "Image à recadrer",
      stageHint: "Les flèches déplacent l’image, Maj la déplace plus loin, + et − zooment.",
      zoom: "Zoom",
      centre: "Recentrer",
      samples: "À sa taille réelle",
      sampleAtRest: "Dans la liste",
      sampleSelected: "Sélectionné",
      opening: "Ouverture de l’image…",
      save: "Enregistrer l’icône",
      saving: "Envoi…",
    },
  },

  settings: {
    userSettings: "Paramètres utilisateur",
    appSettings: "Paramètres de l’application",
    myAccount: "Mon compte",
    profile: "Profil",
    about: "À propos",
    closeSettings: "Fermer les paramètres",
    escapeKey: "Échap",
    signOut: "Se déconnecter",
    signOutFailed: "Impossible de te déconnecter",
    backToUserSettings: "Retour aux paramètres utilisateur",
    webClient: "Client web Mellow",
    sourceCode: "Code source",
    licence: "Mellow est un logiciel libre sous licence AGPL-3.0. Tu peux lire, modifier et partager son code.",
    sourceCodeOnGitHub: "Code source sur GitHub",
  },

  account: {
    displayName: "Nom d’affichage",
    username: "Nom d’utilisateur",
    email: "E-mail",
    password: "Mot de passe",
    editDisplayName: "Modifier le nom d’affichage",
    editUsername: "Modifier le nom d’utilisateur",
    changeDisplayName: "Changer le nom d’affichage",
    changeUsername: "Changer le nom d’utilisateur",
    changePassword: "Changer le mot de passe",
    changingPassword: "Changement du mot de passe…",
    displayNameHelper: "Jusqu’à 32 caractères. C’est le nom que les autres voient dans la discussion.",
    currentPassword: "Mot de passe actuel",
    newPassword: "Nouveau mot de passe",
    confirmNewPassword: "Confirmer le nouveau mot de passe",
    passwordsDiffer: "Ne correspond pas au nouveau mot de passe.",
    wrongCurrentPassword: "le mot de passe actuel est incorrect.",
    displayNameFailed: "Impossible d’enregistrer le nom d’affichage",
    usernameFailed: "Impossible de changer le nom d’utilisateur",
    passwordFailed: "Impossible de changer le mot de passe",
    displayNameSaved: "Nom d’affichage enregistré.",
    usernameSaved: "Nom d’utilisateur enregistré.",
    passwordChanged: "Mot de passe changé. Tes autres appareils sont déconnectés.",
  },

  profile: {
    formats: (maximumSize) =>
      `Les images peuvent être en PNG, JPEG, GIF ou WebP, jusqu’à ${maximumSize}. Les GIF animés restent animés.`,
    tooLarge: (maximumSize) => `l’image dépasse ${maximumSize}. Choisis-en une plus petite.`,
    unreadable: `l’image est illisible, ou dépasse 8${NARROW_NO_BREAK_SPACE}192 pixels de côté ou 600 images d’animation. Choisis-en une autre.`,
    notAPicture: "choisis une image PNG, JPEG, GIF ou WebP.",
    preview: "Aperçu",
    avatar: {
      title: "Avatar",
      emptyText: "Ton initiale s’affiche tant que tu n’as pas ajouté d’image.",
      saved: "Avatar enregistré.",
      removed: "Avatar retiré.",
      change: "Changer l’avatar",
      uploading: "Envoi de l’avatar…",
      remove: "Retirer l’avatar",
      removing: "Retrait de l’avatar…",
      changeFailed: "Impossible de changer l’avatar",
      removeFailed: "Impossible de retirer l’avatar",
    },
    banner: {
      title: "Bannière du profil",
      emptyText: "Un bandeau uni s’affiche tant que tu n’as pas ajouté d’image.",
      saved: "Bannière enregistrée.",
      removed: "Bannière retirée.",
      change: "Changer la bannière",
      uploading: "Envoi de la bannière…",
      remove: "Retirer la bannière",
      removing: "Retrait de la bannière…",
      changeFailed: "Impossible de changer la bannière",
      removeFailed: "Impossible de retirer la bannière",
    },
  },
};
