//! The languages captions are offered in, by their BCP 47 tags. A caption
//! track can hold any valid tag; these are the ones named in menus.

/// Tags and English names, most used first.
pub const LANGUAGES: [(&str, &str); 40] = [
    ("en", "English"),
    ("es", "Spanish"),
    ("pt", "Portuguese"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("tr", "Turkish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("bn", "Bengali"),
    ("ur", "Urdu"),
    ("id", "Indonesian"),
    ("ms", "Malay"),
    ("vi", "Vietnamese"),
    ("th", "Thai"),
    ("tl", "Filipino"),
    ("zh", "Chinese"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("fa", "Persian"),
    ("he", "Hebrew"),
    ("el", "Greek"),
    ("sv", "Swedish"),
    ("no", "Norwegian"),
    ("da", "Danish"),
    ("fi", "Finnish"),
    ("cs", "Czech"),
    ("sk", "Slovak"),
    ("hu", "Hungarian"),
    ("ro", "Romanian"),
    ("bg", "Bulgarian"),
    ("hr", "Croatian"),
    ("sr", "Serbian"),
    ("ca", "Catalan"),
    ("ta", "Tamil"),
    ("sw", "Swahili"),
];

/// The English name of a tag's language, its region and script aside:
/// `pt-BR` is Portuguese.
pub fn name(tag: &str) -> Option<&'static str> {
    let primary = tag.split('-').next()?.to_ascii_lowercase();
    // Norwegian Bokmål and Nynorsk are Norwegian.
    let primary = match primary.as_str() {
        "nb" | "nn" => "no",
        "fil" => "tl",
        other => other,
    };
    (LANGUAGES.iter())
        .find(|(code, _)| *code == primary)
        .map(|(_, name)| *name)
}

/// A tag as people read it: `English`, `Portuguese (pt-BR)`, or the tag.
pub fn label(tag: &str) -> String {
    match name(tag) {
        Some(name) if tag.contains('-') => format!("{name} ({tag})"),
        Some(name) => name.to_owned(),
        None => tag.to_owned(),
    }
}

/// The tag a person typed or a file named, as a track keeps it: trimmed,
/// its language lower case and its region upper case. `None` when it is
/// not a tag.
pub fn tag(text: &str) -> Option<String> {
    let text = text.trim();
    // A language typed by name is its tag.
    if let Some((code, _)) = (LANGUAGES.iter()).find(|(_, name)| name.eq_ignore_ascii_case(text)) {
        return Some((*code).to_owned());
    }
    let mut parts = text.split(['-', '_']);
    let mut tag = parts.next()?.to_ascii_lowercase();
    for part in parts {
        tag.push('-');
        if part.len() == 2 {
            tag.push_str(&part.to_ascii_uppercase());
        } else {
            tag.push_str(part);
        }
    }
    (editor_core::title::language_valid(&tag) && well_formed(&tag)).then_some(tag)
}

/// Whether `tag` has the shape of a BCP 47 tag: a language of two or three
/// letters (five to eight when registered), or `x-` for private use, and
/// subtags of one to eight letters or digits.
fn well_formed(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let language = parts.next().is_some_and(|first| match first.len() {
        1 => first == "x",
        2 | 3 | 5..=8 => first.chars().all(|c| c.is_ascii_alphabetic()),
        _ => false,
    });
    language && parts.all(|part| (1..=8).contains(&part.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_named_and_read_as_typed() {
        assert_eq!(label("en"), "English");
        assert_eq!(label("pt-BR"), "Portuguese (pt-BR)");
        assert_eq!(label("nb"), "Norwegian");
        assert_eq!(label("x-klingon"), "x-klingon");
        assert_eq!(tag(" Spanish ").as_deref(), Some("es"));
        assert_eq!(tag("pt_br").as_deref(), Some("pt-BR"));
        assert_eq!(tag("EN").as_deref(), Some("en"));
        assert_eq!(tag("zh-Hant").as_deref(), Some("zh-Hant"));
        assert_eq!(tag(""), None);
        assert_eq!(tag("en us"), None);
        assert_eq!(tag("1en"), None);
        assert_eq!(tag("Klingonese"), None, "not a language name or a tag");
        assert_eq!(tag("tlh").as_deref(), Some("tlh"));
        assert_eq!(tag("x-klingon").as_deref(), Some("x-klingon"));
        assert_eq!(tag("en-verylongsubtag"), None);
        let mut seen = std::collections::HashSet::new();
        for (code, name) in LANGUAGES {
            assert!(seen.insert(code), "{code} is listed once");
            assert_eq!(tag(code).as_deref(), Some(code), "{name}");
        }
    }
}
