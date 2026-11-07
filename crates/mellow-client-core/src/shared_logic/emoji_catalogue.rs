//! The emoji picker's built-in set (`web/src/lib/emoji-catalogue.ts`): about two hundred common
//! emoji, grouped as on Discord, each with a few English and French search words.
//!
//! shortcut: the table is copied from the web's file by hand (319 entries on 2026-11-07); a
//! change there must be made here too until a generator reads both from one source.

use std::sync::LazyLock;

use super::search_text::fold_for_search;
use super::unicode_classes::is_js_whitespace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmojiCategory {
    People,
    Nature,
    Food,
    Activities,
    Travel,
    Objects,
    Symbols,
    Flags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEmoji {
    /// As sent and shown, variation selector included ("❤️").
    pub emoji: &'static str,
    pub category: EmojiCategory,
    /// Lowercase English and French words, searched together whatever the interface language.
    pub keywords: Vec<&'static str>,
}

/// Every built-in emoji, in category order.
pub static EMOJI_CATALOGUE: LazyLock<Vec<CatalogueEmoji>> = LazyLock::new(|| {
    RAW_CATALOGUE
        .iter()
        .flat_map(|(category, entries)| entries.iter().map(|entry| parse_entry(*category, entry)))
        .collect()
});

/// The keywords folded once, in the catalogue's order.
static FOLDED_KEYWORDS: LazyLock<Vec<Vec<String>>> = LazyLock::new(|| {
    EMOJI_CATALOGUE
        .iter()
        .map(|entry| {
            entry
                .keywords
                .iter()
                .map(|word| fold_for_search(word))
                .collect()
        })
        .collect()
});

/// `emoji english words|mots français`.
fn parse_entry(category: EmojiCategory, entry: &'static str) -> CatalogueEmoji {
    let (emoji, words) = entry.split_once(' ').unwrap_or((entry, ""));
    CatalogueEmoji {
        emoji,
        category,
        keywords: words
            .split([' ', '|'])
            .filter(|word| !word.is_empty())
            .collect(),
    }
}

/// Emoji whose words start with every word of the query, in catalogue order; an exact word comes
/// first. An empty query finds nothing (the picker then shows the categories).
#[must_use]
pub fn search_emoji(query: &str) -> Vec<&'static CatalogueEmoji> {
    let folded_query = fold_for_search(query);
    let terms: Vec<&str> = folded_query
        .split(is_js_whitespace)
        .filter(|term| !term.is_empty())
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut exact = Vec::new();
    let mut prefixed = Vec::new();
    for (entry, words) in EMOJI_CATALOGUE.iter().zip(FOLDED_KEYWORDS.iter()) {
        if !terms
            .iter()
            .all(|term| words.iter().any(|word| word.starts_with(term)))
        {
            continue;
        }
        if terms
            .iter()
            .all(|term| words.iter().any(|word| word == term))
        {
            exact.push(entry);
        } else {
            prefixed.push(entry);
        }
    }
    exact.extend(prefixed);
    exact
}

const RAW_CATALOGUE: &[(EmojiCategory, &[&str])] = &[
    (
        EmojiCategory::People,
        &[
            "😀 grinning smile happy|sourire content heureux",
            "😃 smiley happy|content joyeux",
            "😄 smile laugh|sourire rire",
            "😁 grin beaming|sourire dents",
            "😆 laughing lol|rire mdr",
            "😅 sweat smile relief|soulagé sueur",
            "🤣 rofl rolling laughing|mort de rire mdr",
            "😂 joy tears laughing lol|larmes de joie rire mdr",
            "🙂 slight smile|léger sourire",
            "🙃 upside down|à l'envers",
            "😉 wink|clin d'œil",
            "😊 blush happy|rougir content",
            "😇 innocent halo angel|innocent ange auréole",
            "🥰 love hearts adore|amoureux cœurs",
            "😍 heart eyes love|yeux cœur amour",
            "🤩 star struck wow|étoiles ébloui",
            "😘 kiss blow|bisou baiser",
            "😋 yum delicious|miam délicieux",
            "😛 tongue|langue",
            "😜 wink tongue silly|langue clin d'œil",
            "🤪 zany crazy|fou loufoque",
            "🤗 hug hugging|câlin",
            "🤭 oops giggle|oups pouffer",
            "🤫 shush quiet secret|chut secret",
            "🤔 thinking hmm|réfléchir hmm",
            "🤐 zipper mouth|bouche cousue",
            "🤨 raised eyebrow suspicious|sourcil sceptique",
            "😐 neutral|neutre",
            "😑 expressionless|sans expression",
            "😶 no mouth speechless|sans voix",
            "😏 smirk|sourire en coin",
            "😒 unamused meh|blasé bof",
            "🙄 eye roll|lever les yeux",
            "😬 grimace awkward|grimace gêné",
            "😌 relieved calm|soulagé calme",
            "😔 pensive sad|pensif triste",
            "😪 sleepy|endormi",
            "😴 sleeping zzz|dormir dodo",
            "😷 mask sick|masque malade",
            "🤒 thermometer sick ill|fièvre malade",
            "🤢 nauseated sick|nausée malade",
            "🤮 vomit|vomir",
            "🥵 hot|chaud",
            "🥶 cold freezing|froid gelé",
            "🥴 woozy dizzy|étourdi pompette",
            "😵 dizzy|sonné",
            "🤯 mind blown|époustouflé explosion",
            "🤠 cowboy|cowboy",
            "🥳 party celebrate|fête célébrer",
            "😎 cool sunglasses|cool lunettes de soleil",
            "🤓 nerd geek|intello geek",
            "🧐 monocle curious|monocle curieux",
            "😕 confused|confus",
            "😟 worried|inquiet",
            "🙁 frown|mécontent",
            "😮 open mouth surprised wow|surpris bouche bée",
            "😲 astonished shocked|stupéfait choqué",
            "🥺 pleading puppy eyes|suppliant yeux de chien battu",
            "😢 cry sad tear|pleurer triste larme",
            "😭 sob crying|sangloter pleurer",
            "😱 scream fear|cri peur",
            "😤 triumph huff|fier souffle",
            "😡 angry rage|en colère rage",
            "😠 mad angry|fâché",
            "🤬 cursing swear|jurer injures",
            "😈 devil smiling|diable",
            "💀 skull dead|crâne mort",
            "💩 poop|caca",
            "🤡 clown|clown",
            "👻 ghost boo|fantôme",
            "👽 alien|extraterrestre",
            "🤖 robot|robot",
            "👋 wave hello bye|salut coucou au revoir",
            "🤚 raised back hand|main levée",
            "✋ hand stop high five|main stop",
            "👌 ok perfect|ok parfait",
            "🤌 pinched fingers|doigts pincés",
            "✌️ victory peace|victoire paix",
            "🤞 crossed fingers luck|doigts croisés chance",
            "🤟 love you|je t'aime",
            "🤘 rock horns|rock cornes",
            "🤙 call me|appelle-moi",
            "👈 point left|gauche",
            "👉 point right|droite",
            "👆 point up|haut",
            "👇 point down|bas",
            "👍 thumbs up yes like|pouce oui j'aime",
            "👎 thumbs down no dislike|pouce bas non",
            "✊ fist|poing",
            "👊 punch fist bump|coup de poing check",
            "👏 clap applause bravo|applaudir bravo",
            "🙌 raised hands hooray|hourra",
            "👐 open hands|mains ouvertes",
            "🤝 handshake deal|poignée de main accord",
            "🙏 pray thanks please|prier merci s'il te plaît",
            "💪 muscle strong|muscle fort",
            "👀 eyes look|yeux regarder",
            "🧠 brain smart|cerveau malin",
            "🫡 salute|salut militaire",
        ],
    ),
    (
        EmojiCategory::Nature,
        &[
            "🐶 dog puppy|chien chiot",
            "🐱 cat kitten|chat chaton",
            "🐭 mouse|souris",
            "🐰 rabbit bunny|lapin",
            "🦊 fox|renard",
            "🐻 bear|ours",
            "🐼 panda|panda",
            "🐨 koala|koala",
            "🐯 tiger|tigre",
            "🦁 lion|lion",
            "🐮 cow|vache",
            "🐷 pig|cochon",
            "🐸 frog|grenouille",
            "🐵 monkey|singe",
            "🐔 chicken|poule",
            "🐧 penguin|pingouin manchot",
            "🐦 bird|oiseau",
            "🦆 duck|canard",
            "🦉 owl|chouette hibou",
            "🐝 bee|abeille",
            "🦋 butterfly|papillon",
            "🐌 snail slow|escargot lent",
            "🐢 turtle slow|tortue lent",
            "🐍 snake|serpent",
            "🐙 octopus|pieuvre poulpe",
            "🐳 whale|baleine",
            "🐬 dolphin|dauphin",
            "🦄 unicorn|licorne",
            "🌸 cherry blossom flower|fleur de cerisier",
            "🌹 rose|rose",
            "🌻 sunflower|tournesol",
            "🌷 tulip|tulipe",
            "🌱 seedling plant|pousse plante",
            "🌲 tree evergreen|sapin arbre",
            "🍀 four leaf clover luck|trèfle chance",
            "🍁 maple leaf autumn|érable automne",
            "🌵 cactus|cactus",
            "🌈 rainbow|arc-en-ciel",
            "☀️ sun sunny|soleil",
            "🌙 moon night|lune nuit",
            "⭐ star|étoile",
            "🌟 glowing star|étoile brillante",
            "⚡ lightning zap|éclair",
            "🔥 fire lit hot|feu chaud",
            "❄️ snowflake cold|flocon froid",
            "🌊 wave ocean|vague océan",
            "☔ umbrella rain|parapluie pluie",
        ],
    ),
    (
        EmojiCategory::Food,
        &[
            "🍎 apple|pomme",
            "🍐 pear|poire",
            "🍊 orange|orange",
            "🍋 lemon|citron",
            "🍌 banana|banane",
            "🍉 watermelon|pastèque",
            "🍇 grapes|raisin",
            "🍓 strawberry|fraise",
            "🍒 cherries|cerises",
            "🍑 peach|pêche",
            "🥑 avocado|avocat",
            "🥕 carrot|carotte",
            "🌽 corn|maïs",
            "🥐 croissant|croissant",
            "🥖 baguette bread|baguette pain",
            "🧀 cheese|fromage",
            "🥚 egg|œuf",
            "🥞 pancakes|crêpes",
            "🍔 burger|burger hamburger",
            "🍟 fries|frites",
            "🍕 pizza|pizza",
            "🌭 hot dog|hot-dog",
            "🌮 taco|taco",
            "🍝 spaghetti pasta|spaghetti pâtes",
            "🍜 ramen noodles|ramen nouilles",
            "🍣 sushi|sushi",
            "🍦 ice cream|glace",
            "🍩 doughnut donut|beignet donut",
            "🍪 cookie|cookie biscuit",
            "🎂 birthday cake|gâteau anniversaire",
            "🍫 chocolate|chocolat",
            "🍿 popcorn|pop-corn",
            "☕ coffee|café",
            "🍵 tea|thé",
            "🍺 beer|bière",
            "🍻 cheers beers|santé bières",
            "🍷 wine|vin",
            "🥂 toast cheers champagne|trinquer santé",
        ],
    ),
    (
        EmojiCategory::Activities,
        &[
            "⚽ soccer football|football foot",
            "🏀 basketball|basket",
            "🏈 american football|football américain",
            "🎾 tennis|tennis",
            "🏐 volleyball|volley",
            "🏓 ping pong|ping-pong",
            "🚴 cycling bike|vélo cyclisme",
            "🏃 running|course",
            "🏆 trophy win|trophée victoire",
            "🥇 gold medal first|médaille d'or premier",
            "🎉 party tada celebrate|fête bravo célébrer",
            "🎊 confetti|confettis",
            "🎈 balloon|ballon",
            "🎁 gift present|cadeau",
            "🎮 video game controller|jeu vidéo manette",
            "🕹️ joystick|joystick",
            "🎲 dice game|dé jeu",
            "♟️ chess pawn|échecs pion",
            "🧩 puzzle piece|puzzle pièce",
            "🎯 bullseye target|cible",
            "🎨 art palette|art palette peinture",
            "🎬 movie clapper|cinéma clap",
            "🎤 microphone karaoke|micro karaoké",
            "🎧 headphones|casque",
            "🎵 music note|musique note",
            "🎸 guitar|guitare",
            "🎹 piano keyboard|piano clavier",
        ],
    ),
    (
        EmojiCategory::Travel,
        &[
            "🚗 car|voiture",
            "🚕 taxi|taxi",
            "🚌 bus|bus",
            "🚲 bicycle|vélo bicyclette",
            "🚂 train|train",
            "✈️ airplane plane|avion",
            "🚀 rocket launch ship it|fusée lancement",
            "🛸 ufo|ovni soucoupe",
            "⛵ sailboat|voilier",
            "🏠 house home|maison",
            "🏢 office building|bureau immeuble",
            "🏰 castle|château",
            "🗼 tower|tour",
            "🗽 statue of liberty|statue de la liberté",
            "⛰️ mountain|montagne",
            "🏖️ beach|plage",
            "🏕️ camping|camping",
            "🌍 earth globe world|terre monde globe",
            "🗺️ map|carte",
            "🌃 night city|ville nuit",
        ],
    ),
    (
        EmojiCategory::Objects,
        &[
            "⌚ watch|montre",
            "📱 phone mobile|téléphone portable",
            "💻 laptop computer|ordinateur portable",
            "⌨️ keyboard|clavier",
            "🖥️ desktop computer|ordinateur écran",
            "🖱️ mouse|souris",
            "💾 floppy save|disquette sauvegarder",
            "📷 camera photo|appareil photo",
            "📺 television tv|télévision télé",
            "💡 bulb idea|ampoule idée",
            "🔦 flashlight|lampe torche",
            "📚 books|livres",
            "📖 book open|livre ouvert",
            "📝 memo note|mémo note",
            "✏️ pencil|crayon",
            "📌 pin pushpin|punaise épingle",
            "📎 paperclip|trombone",
            "✂️ scissors|ciseaux",
            "📅 calendar date|calendrier date",
            "📈 chart up increase|graphique hausse",
            "📉 chart down decrease|graphique baisse",
            "📦 package box|colis boîte",
            "✉️ envelope mail|enveloppe courrier",
            "🔒 lock locked|cadenas verrouillé",
            "🔑 key|clé",
            "🔨 hammer|marteau",
            "🔧 wrench fix|clé à molette réparer",
            "⚙️ gear settings|engrenage réglages",
            "🧪 test tube experiment|éprouvette expérience",
            "🔔 bell notification|cloche notification",
            "💰 money bag|sac d'argent",
            "💸 money flying|argent qui s'envole",
            "🛒 shopping cart|caddie courses",
            "🍾 champagne bottle|bouteille de champagne",
        ],
    ),
    (
        EmojiCategory::Symbols,
        &[
            "❤️ red heart love|cœur rouge amour",
            "🧡 orange heart|cœur orange",
            "💛 yellow heart|cœur jaune",
            "💚 green heart|cœur vert",
            "💙 blue heart|cœur bleu",
            "💜 purple heart|cœur violet",
            "🖤 black heart|cœur noir",
            "🤍 white heart|cœur blanc",
            "💔 broken heart|cœur brisé",
            "💕 two hearts|deux cœurs",
            "💖 sparkling heart|cœur étincelant",
            "💯 hundred perfect|cent parfait",
            "✨ sparkles|étincelles",
            "💥 boom collision|boum collision",
            "💫 dizzy star|étourdi étoile",
            "💬 speech bubble|bulle de dialogue",
            "💤 zzz sleep|zzz sommeil",
            "✅ check done yes|coche fait oui",
            "☑️ ballot check|case cochée",
            "✔️ check mark|coche",
            "❌ cross no wrong|croix non faux",
            "❓ question|question",
            "❗ exclamation|exclamation",
            "⚠️ warning|attention avertissement",
            "🚫 prohibited forbidden|interdit",
            "⛔ no entry|sens interdit",
            "♻️ recycle|recyclage",
            "➕ plus add|plus ajouter",
            "➖ minus|moins",
            "🆗 ok button|bouton ok",
            "🆕 new|nouveau",
            "🆒 cool button|bouton cool",
            "🔴 red circle|cercle rouge",
            "🟢 green circle|cercle vert",
            "🔵 blue circle|cercle bleu",
            "1️⃣ one 1|un 1",
            "2️⃣ two 2|deux 2",
            "3️⃣ three 3|trois 3",
            "#️⃣ hash number|dièse",
        ],
    ),
    (
        EmojiCategory::Flags,
        &[
            "🏁 chequered flag finish|drapeau à damier arrivée",
            "🚩 red flag|drapeau rouge",
            "🏳️‍🌈 rainbow flag pride|drapeau arc-en-ciel fierté",
            "🏴‍☠️ pirate flag|drapeau pirate",
            "🇫🇷 france french|france français",
            "🇧🇪 belgium|belgique",
            "🇨🇭 switzerland|suisse",
            "🇨🇦 canada|canada",
            "🇬🇧 united kingdom uk|royaume-uni",
            "🇺🇸 united states usa|états-unis",
            "🇩🇪 germany|allemagne",
            "🇪🇸 spain|espagne",
            "🇮🇹 italy|italie",
            "🇯🇵 japan|japon",
            "🇪🇺 european union|union européenne",
        ],
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn found(query: &str) -> Vec<&'static str> {
        search_emoji(query)
            .iter()
            .map(|entry| entry.emoji)
            .collect()
    }

    #[test]
    fn holds_about_two_hundred_distinct_emoji_each_with_words_in_both_languages() {
        let distinct: HashSet<&str> = EMOJI_CATALOGUE.iter().map(|entry| entry.emoji).collect();
        assert_eq!(distinct.len(), EMOJI_CATALOGUE.len());
        assert!(EMOJI_CATALOGUE.len() >= 200);
        for entry in EMOJI_CATALOGUE.iter() {
            assert!(entry.keywords.len() >= 2, "{}", entry.emoji);
        }
    }

    #[test]
    fn is_searched_in_english_and_french_accents_and_case_aside_exact_words_first() {
        assert!(found("heart").contains(&"❤️"));
        assert!(found("Coeur").contains(&"❤️"));
        assert_eq!(found("cœur rouge"), ["❤️"]);
        assert_eq!(found("pouce").first(), Some(&"👍"));
        assert_eq!(found("cat").first(), Some(&"🐱"));
        assert_eq!(found("   "), Vec::<&str>::new());
        assert_eq!(found("zzzzqx"), Vec::<&str>::new());
    }
}
