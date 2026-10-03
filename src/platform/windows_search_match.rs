//! Small, bounded filename/app-name matcher; it never reads file contents.
pub fn fold(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).filter_map(|c| Some(match c {
        '\u{0300}'..='\u{036f}' => return None,
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e', 'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o', 'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n', 'ç' => 'c', other => other,
    })).collect()
}

fn near(a: &str, b: &str) -> bool {
    let (a, b): (Vec<_>, Vec<_>) = (a.chars().collect(), b.chars().collect());
    if a.len() < 4 || a.len().abs_diff(b.len()) > 1 { return false; }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] { i += 1; j += 1; continue; }
        edits += 1;
        if edits > 1 { return false; }
        if a.len() == b.len() { i += 1; j += 1; }
        else if a.len() > b.len() { i += 1; } else { j += 1; }
    }
    edits + usize::from(i < a.len() || j < b.len()) <= 1
}

pub fn score(name: &str, query: &str) -> Option<u32> {
    let name = fold(name);
    let query = fold(query);
    let query = query.trim();
    if query.is_empty() { return Some(0); }
    if name == query { return Some(1000); }
    if name.starts_with(query) { return Some(900); }
    if name.contains(query) { return Some(800); }
    let words: Vec<_> = name.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect();
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
    if query.chars().count() >= 2 && initials == query { return Some(750); }
    let mut score = 0;
    let mut count = 0;
    for token in query.split_whitespace() {
        score += if words.iter().any(|w| w.starts_with(token)) { 700 }
            else if name.contains(token) { 600 }
            else if words.iter().any(|w| near(token, w)) { 400 }
            else { return None; };
        count += 1;
    }
    Some(score / count.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn words_accents_acronyms_and_typos() {
        assert!(score("Canción del verano.txt", "verano cancion").is_some());
        assert!(score("CAFE\u{301}.txt", "café").is_some());
        assert!(score("Visual Studio Code", "vsc").is_some());
        assert!(score("Spotify", "spotfy").is_some());
        assert!(score("日本語.txt", "日本語").is_some());
        assert!(score("Spotify", "spotszzz").is_none());
        assert!(score("cat", "bat").is_none());
        assert!(score("Spotify", "spotify") > score("Spotify", "spotfy"));
    }
}
