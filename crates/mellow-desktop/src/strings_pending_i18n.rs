//! The interface strings of the sign-in screens and the connection banner, in English and French.
//!
//! TODO(i18n catalogue track): replace this module with the catalogue generated from
//! `web/src/i18n/en.ts` and `fr.ts` (ADR-0051 "Design"). Every string the web client has is copied
//! from its catalogue word for word (the key is in the comment); the instance address screen,
//! which the web client does not have, follows DESIGN.md §5.1 in English and the catalogue's voice
//! (tutoiement, `\u{a0}` before `:`, `\u{202f}` before `?`) in French.

use gpui::{App, Global};
use mellow_protocol::ApiErrorCode;

const NO_BREAK_SPACE: char = '\u{a0}';

/// The strings of the language the app speaks, as a GPUI global.
#[derive(Debug, Clone, Copy)]
pub struct ActiveStrings(pub &'static Strings);

impl Global for ActiveStrings {}

/// Installed by `start` before any view renders; tests install it themselves.
#[must_use]
pub fn strings(cx: &App) -> &'static Strings {
    cx.global::<ActiveStrings>().0
}

/// Why a request failed, as the end of a sentence (`errors.reasons`).
#[derive(Debug)]
pub struct ErrorReasons {
    pub network: &'static str,
    pub invalid_credentials: &'static str,
    pub username_taken: &'static str,
    pub email_taken: &'static str,
    pub registration_closed: &'static str,
    pub validation_failed: &'static str,
    pub rate_limited: &'static str,
    pub server_busy: &'static str,
    pub missing_permissions: &'static str,
    pub not_found: &'static str,
    pub invalid_invite: &'static str,
    pub banned: &'static str,
    pub owner_cannot_leave: &'static str,
    pub limit_reached: &'static str,
    pub file_too_large: &'static str,
    /// A file picked on this computer that could not be read (the web has no such case).
    pub file_unreadable: &'static str,
    pub conflict: &'static str,
    pub invalid_two_factor_code: &'static str,
}

impl ErrorReasons {
    #[must_use]
    pub fn for_code(&self, code: ApiErrorCode) -> Option<&'static str> {
        match code {
            ApiErrorCode::InvalidCredentials => Some(self.invalid_credentials),
            ApiErrorCode::UsernameTaken => Some(self.username_taken),
            ApiErrorCode::EmailTaken => Some(self.email_taken),
            ApiErrorCode::RegistrationClosed => Some(self.registration_closed),
            ApiErrorCode::ValidationFailed => Some(self.validation_failed),
            ApiErrorCode::RateLimited => Some(self.rate_limited),
            ApiErrorCode::ServerBusy => Some(self.server_busy),
            ApiErrorCode::MissingPermissions => Some(self.missing_permissions),
            ApiErrorCode::NotFound => Some(self.not_found),
            ApiErrorCode::InvalidInvite => Some(self.invalid_invite),
            ApiErrorCode::Banned => Some(self.banned),
            ApiErrorCode::OwnerCannotLeave => Some(self.owner_cannot_leave),
            ApiErrorCode::LimitReached => Some(self.limit_reached),
            ApiErrorCode::Conflict => Some(self.conflict),
            ApiErrorCode::InvalidTwoFactorCode => Some(self.invalid_two_factor_code),
            ApiErrorCode::Unauthorized | ApiErrorCode::AccountDisabled | ApiErrorCode::Internal => {
                None
            }
        }
    }
}

#[derive(Debug)]
pub struct Strings {
    // errors
    /// `errors.withReason`.
    pub error_with_reason: fn(&str, &str) -> String,
    pub error_unknown_reason: &'static str,
    pub error_reasons: ErrorReasons,
    // app
    pub app_unreachable_title: &'static str,
    pub app_unreachable_body: &'static str,
    // connection
    pub connection_connecting: &'static str,
    pub connection_reconnecting: &'static str,
    pub connection_still_reconnecting: &'static str,
    pub connection_retry_now: &'static str,
    pub connection_offline: &'static str,
    pub connection_back_online: &'static str,
    // signIn
    pub sign_in_title: fn(&str) -> String,
    pub register_title: fn(&str) -> String,
    pub sign_in_subtitle: &'static str,
    pub register_subtitle: &'static str,
    pub email: &'static str,
    pub username: &'static str,
    pub username_helper: &'static str,
    pub password: &'static str,
    pub password_length_helper: fn(usize) -> String,
    pub login: &'static str,
    pub sign_in: &'static str,
    pub signing_in: &'static str,
    pub create_account: &'static str,
    pub creating_account: &'static str,
    pub has_account: &'static str,
    pub has_no_account: &'static str,
    pub create_an_account: &'static str,
    pub sign_in_failed: &'static str,
    pub register_failed: &'static str,
    pub session_ended: fn(&str) -> String,
    pub login_missing: &'static str,
    pub password_missing: &'static str,
    pub email_missing: &'static str,
    pub email_malformed: &'static str,
    pub username_missing: &'static str,
    pub new_password_missing: &'static str,
    // security.twoFactorHeading and security.signIn
    pub two_factor_heading: &'static str,
    pub two_factor_subtitle: &'static str,
    pub two_factor_recovery_subtitle: &'static str,
    pub two_factor_code: &'static str,
    pub two_factor_recovery_code: &'static str,
    pub two_factor_code_missing: &'static str,
    pub two_factor_recovery_code_missing: &'static str,
    pub two_factor_verify: &'static str,
    pub two_factor_verifying: &'static str,
    pub two_factor_use_recovery_code: &'static str,
    pub two_factor_use_authenticator: &'static str,
    pub two_factor_back_to_sign_in: &'static str,
    pub two_factor_verify_failed: &'static str,
    pub two_factor_expired: &'static str,
    pub two_factor_codes_locked: &'static str,
    // settings
    pub sign_out: &'static str,
    pub sign_out_failed: &'static str,
    // Desktop only: the instance address screen (DESIGN.md §5.1).
    pub instance_title: &'static str,
    pub instance_subtitle: &'static str,
    pub instance_address: &'static str,
    pub instance_address_placeholder: &'static str,
    pub instance_continue: &'static str,
    pub instance_checking: fn(&str) -> String,
    pub instance_address_missing: &'static str,
    pub instance_address_invalid: &'static str,
    pub instance_unreachable: fn(&str) -> String,
    pub instance_not_mellow: fn(&str) -> String,
    /// Host, the instance's version, the oldest version the app supports.
    pub instance_too_old: fn(&str, &str, &str) -> String,
    /// The instance row under the sign-in title: its label, its button, the button's name.
    pub instance_row_label: &'static str,
    pub change_instance: &'static str,
    pub change_instance_label: &'static str,
    /// Why a picture was refused before sending: its megapixels and longest side allowed.
    pub picture_too_many_pixels: fn(u32, u32) -> String,
}

pub static ENGLISH: Strings = Strings {
    error_with_reason: |summary, reason| format!("{summary}: {reason}"),
    error_unknown_reason: "something went wrong. Try again.",
    error_reasons: ErrorReasons {
        network: "the instance could not be reached. Try again.",
        invalid_credentials: "the login or the password is wrong.",
        username_taken: "this name is taken.",
        email_taken: "this email already has an account.",
        registration_closed: "this instance does not accept new accounts.",
        validation_failed: "the instance refused what was entered.",
        rate_limited: "too many attempts. Wait a moment and try again.",
        server_busy: "the instance is busy. Try again in a moment.",
        missing_permissions: "you don't have the permission to do that here.",
        not_found: "it no longer exists, or you can't see it.",
        invalid_invite: "this invite is unknown, expired or used up.",
        banned: "you are banned from this server.",
        owner_cannot_leave: "the owner cannot leave their own server.",
        limit_reached: "a limit of this instance is reached.",
        file_too_large: "the file is larger than this instance accepts.",
        file_unreadable: "this file can't be read on this computer.",
        conflict: "someone else changed it at the same time. Try again.",
        invalid_two_factor_code: "the code is wrong, already used or too old.",
    },
    app_unreachable_title: "This instance can't be reached right now.",
    app_unreachable_body: "Check your connection, then try again.",
    connection_connecting: "Connecting…",
    connection_reconnecting: "Connection lost. Reconnecting…",
    connection_still_reconnecting: "Still trying to reconnect. Messages you send will wait until you're back.",
    connection_retry_now: "Retry now",
    connection_offline: "You're offline. Showing saved messages.",
    connection_back_online: "Back online",
    sign_in_title: |instance| format!("Sign in to {instance}"),
    register_title: |instance| format!("Create an account on {instance}"),
    sign_in_subtitle: "Your account is hosted on this instance.",
    register_subtitle: "Your account will live on this instance.",
    email: "Email",
    username: "Name",
    username_helper: "2 to 32 characters: letters, digits, spaces, dots, dashes and underscores. Nobody else here can have it, whatever the capitals or accents.",
    password: "Password",
    password_length_helper: |minimum| format!("At least {minimum} characters."),
    login: "Name or email",
    sign_in: "Sign in",
    signing_in: "Signing in…",
    create_account: "Create account",
    creating_account: "Creating account…",
    has_account: "Already have an account?",
    has_no_account: "No account yet?",
    create_an_account: "Create an account",
    sign_in_failed: "Couldn't sign in",
    register_failed: "Couldn't create your account",
    session_ended: |instance| {
        format!("Your session on {instance} has ended. Sign in again to keep chatting.")
    },
    login_missing: "Enter your name or email.",
    password_missing: "Enter your password.",
    email_missing: "Enter your email.",
    email_malformed: "Enter an email address, like ana@example.org.",
    username_missing: "Choose a name.",
    new_password_missing: "Choose a password.",
    two_factor_heading: "Two-factor sign-in",
    two_factor_subtitle: "Open your authenticator app and enter the code it shows for this account.",
    two_factor_recovery_subtitle: "Enter one of the recovery codes you saved. Each works once.",
    two_factor_code: "Code",
    two_factor_recovery_code: "Recovery code",
    two_factor_code_missing: "Enter the 6-digit code.",
    two_factor_recovery_code_missing: "Enter a recovery code, like abcd-efgh-ijkl-mnop.",
    two_factor_verify: "Verify",
    two_factor_verifying: "Verifying…",
    two_factor_use_recovery_code: "Use a recovery code",
    two_factor_use_authenticator: "Use the authenticator app",
    two_factor_back_to_sign_in: "Back to sign in",
    two_factor_verify_failed: "Couldn't sign in",
    two_factor_expired: "This sign-in took too long. Enter your password again.",
    two_factor_codes_locked: "Too many wrong codes: app codes are refused for a while. Try again later or use a recovery code.",
    sign_out: "Sign out",
    sign_out_failed: "Couldn't sign out",
    instance_title: "Chat that you can host yourself.",
    instance_subtitle: "Connect to a Mellow instance to sign in or create an account.",
    instance_address: "Instance address",
    instance_address_placeholder: "chat.example.org",
    instance_continue: "Continue",
    instance_checking: |host| format!("Checking {host}…"),
    instance_address_missing: "Enter the address of an instance.",
    instance_address_invalid: "That doesn't look like an instance address. Try something like chat.example.org.",
    instance_unreachable: |host| {
        format!(
            "We couldn't reach {host}. Check the address, or try again if your connection is down."
        )
    },
    instance_not_mellow: |host| {
        format!(
            "{host} is online but doesn't look like a Mellow instance. Check the address with the person who invited you."
        )
    },
    instance_too_old: |host, version, minimum| {
        format!(
            "{host} runs Mellow {version}. This app needs {minimum} or later. Ask the instance admin to update."
        )
    },
    instance_row_label: "Instance",
    change_instance: "Change",
    change_instance_label: "Change instance",
    picture_too_many_pixels: |megapixels, side| {
        format!("Pictures can be up to {megapixels} megapixels and {side} pixels on a side.")
    },
};

pub static FRENCH: Strings = Strings {
    error_with_reason: |summary, reason| format!("{summary}{NO_BREAK_SPACE}: {reason}"),
    error_unknown_reason: "un problème est survenu. Réessaie.",
    error_reasons: ErrorReasons {
        network: "l’instance est injoignable. Réessaie.",
        invalid_credentials: "l’identifiant ou le mot de passe est incorrect.",
        username_taken: "ce nom est déjà pris.",
        email_taken: "un compte utilise déjà cette adresse e-mail.",
        registration_closed: "cette instance n’accepte pas de nouveaux comptes.",
        validation_failed: "l’instance a refusé ce qui a été saisi.",
        rate_limited: "trop de tentatives. Patiente un instant, puis réessaie.",
        server_busy: "l’instance est surchargée. Réessaie dans un instant.",
        missing_permissions: "tu n’as pas la permission de faire ça ici.",
        not_found: "ça n’existe plus, ou tu ne peux pas le voir.",
        invalid_invite: "cette invitation est inconnue, expirée ou épuisée.",
        banned: "un bannissement t’empêche d’accéder à ce serveur.",
        owner_cannot_leave: "tu ne peux pas quitter un serveur qui t’appartient.",
        limit_reached: "une limite de cette instance est atteinte.",
        file_too_large: "le fichier dépasse la taille acceptée par cette instance.",
        file_unreadable: "ce fichier ne peut pas être lu sur cet ordinateur.",
        conflict: "quelqu’un d’autre l’a modifié au même moment. Réessaie.",
        invalid_two_factor_code: "le code est faux, déjà utilisé ou trop ancien.",
    },
    app_unreachable_title: "Impossible de joindre cette instance pour le moment.",
    app_unreachable_body: "Vérifie ta connexion, puis réessaie.",
    connection_connecting: "Connexion…",
    connection_reconnecting: "Connexion perdue. Reconnexion…",
    connection_still_reconnecting: "Toujours en reconnexion. Tes messages partiront à ton retour.",
    connection_retry_now: "Réessayer maintenant",
    connection_offline: "Tu es hors ligne. Les messages enregistrés restent lisibles.",
    connection_back_online: "De retour en ligne",
    sign_in_title: |instance| format!("Se connecter à {instance}"),
    register_title: |instance| format!("Créer un compte sur {instance}"),
    sign_in_subtitle: "Ton compte est hébergé sur cette instance.",
    register_subtitle: "Ton compte sera hébergé sur cette instance.",
    email: "E-mail",
    username: "Nom",
    username_helper: "De 2 à 32 caractères\u{a0}: lettres, chiffres, espaces, points, tirets et tirets bas. Personne d’autre ici ne peut l’avoir, quelles que soient les majuscules ou les accents.",
    password: "Mot de passe",
    password_length_helper: |minimum| format!("Au moins {minimum} caractères."),
    login: "Nom ou e-mail",
    sign_in: "Se connecter",
    signing_in: "Connexion…",
    create_account: "Créer le compte",
    creating_account: "Création du compte…",
    has_account: "Tu as déjà un compte\u{202f}?",
    has_no_account: "Pas encore de compte\u{202f}?",
    create_an_account: "Créer un compte",
    sign_in_failed: "Impossible de te connecter",
    register_failed: "Impossible de créer ton compte",
    session_ended: |instance| {
        format!("Ta session sur {instance} a pris fin. Reconnecte-toi pour continuer à discuter.")
    },
    login_missing: "Entre ton nom ou ton e-mail.",
    password_missing: "Entre ton mot de passe.",
    email_missing: "Entre ton e-mail.",
    email_malformed: "Entre une adresse e-mail, comme ana@exemple.fr.",
    username_missing: "Choisis un nom.",
    new_password_missing: "Choisis un mot de passe.",
    two_factor_heading: "Connexion en deux étapes",
    two_factor_subtitle: "Ouvre ton application d’authentification et saisis le code affiché pour ce compte.",
    two_factor_recovery_subtitle: "Saisis l’un des codes de secours que tu as gardés. Chacun ne fonctionne qu’une fois.",
    two_factor_code: "Code",
    two_factor_recovery_code: "Code de secours",
    two_factor_code_missing: "Saisis le code à 6 chiffres.",
    two_factor_recovery_code_missing: "Saisis un code de secours, comme abcd-efgh-ijkl-mnop.",
    two_factor_verify: "Vérifier",
    two_factor_verifying: "Vérification…",
    two_factor_use_recovery_code: "Utiliser un code de secours",
    two_factor_use_authenticator: "Utiliser l’application d’authentification",
    two_factor_back_to_sign_in: "Retour à la connexion",
    two_factor_verify_failed: "Impossible de te connecter",
    two_factor_expired: "Cette connexion a pris trop de temps. Saisis à nouveau ton mot de passe.",
    two_factor_codes_locked: "Trop de codes erronés\u{a0}: les codes de l’application sont refusés pour un moment. Réessaie plus tard ou utilise un code de secours.",
    sign_out: "Se déconnecter",
    sign_out_failed: "Impossible de te déconnecter",
    instance_title: "Le chat que tu peux héberger toi-même.",
    instance_subtitle: "Rejoins une instance Mellow pour te connecter ou créer un compte.",
    instance_address: "Adresse de l’instance",
    instance_address_placeholder: "chat.exemple.fr",
    instance_continue: "Continuer",
    instance_checking: |host| format!("Vérification de {host}…"),
    instance_address_missing: "Entre l’adresse d’une instance.",
    instance_address_invalid: "Ça ne ressemble pas à une adresse d’instance. Essaie par exemple chat.exemple.fr.",
    instance_unreachable: |host| {
        format!(
            "Impossible de joindre {host}. Vérifie l’adresse, ou réessaie si ta connexion est coupée."
        )
    },
    instance_not_mellow: |host| {
        format!(
            "{host} répond, mais ne ressemble pas à une instance Mellow. Vérifie l’adresse auprès de la personne qui t’a invité."
        )
    },
    instance_too_old: |host, version, minimum| {
        format!(
            "{host} utilise Mellow {version}. Cette application demande la version {minimum} ou plus récente. Demande à l’admin de l’instance de la mettre à jour."
        )
    },
    instance_row_label: "Instance",
    change_instance: "Changer",
    change_instance_label: "Changer d’instance",
    picture_too_many_pixels: |megapixels, side| {
        // A thin no-break space groups the thousands, as the catalogue writes numbers.
        let side = side.to_string();
        let grouped = match side.len() {
            4.. => format!(
                "{}\u{202f}{}",
                &side[..side.len() - 3],
                &side[side.len() - 3..]
            ),
            _ => side,
        };
        format!(
            "Les images peuvent faire jusqu’à {megapixels}{NO_BREAK_SPACE}mégapixels et {grouped}{NO_BREAK_SPACE}pixels de côté."
        )
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn french_puts_a_no_break_space_before_the_colon() {
        assert_eq!(
            (FRENCH.error_with_reason)("Impossible de te connecter", "raison."),
            "Impossible de te connecter\u{a0}: raison."
        );
        assert_eq!(
            (ENGLISH.error_with_reason)("Couldn't sign in", "reason."),
            "Couldn't sign in: reason."
        );
    }
}
