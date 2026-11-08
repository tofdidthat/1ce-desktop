//! Safe, pure filename *preview* logic. Nothing here renames/moves/deletes files.
//! A filename is never finalized automatically when a tonal key is uncertain.
use serde::Serialize;
use std::collections::HashSet;

pub const DEFAULT_TEMPLATE: &str = "{name} {bpm} {key} {producer}";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamePreview {
    pub proposed: String,
    pub beat_name: String,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub mode: Option<String>,
    pub producer: Option<String>,
    pub missing_bpm: bool,
    pub missing_key: bool,
    pub bpm_conflict: bool,
    pub needs_review: bool,
}

fn parse_bpm(word: &str) -> Option<f64> {
    let lower = word.to_ascii_lowercase();
    let stripped = lower.strip_suffix("bpm").unwrap_or(&lower);
    if stripped.is_empty() || !stripped.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let v: f64 = stripped.parse().ok()?;
    if v.is_finite() && (30.0..=300.0).contains(&v) { Some(v) } else { None }
}

/// Returns spelling and mode if this is an exact musical note token.
/// Supports A..G, accidentals # / b and concise m / min / maj suffixes.
fn parse_key(word: &str) -> Option<(String, Option<String>, bool)> {
    let lower = word.to_ascii_lowercase();
    let mut chars = lower.chars();
    let root = chars.next()?;
    if !('a'..='g').contains(&root) { return None; }
    let rest = chars.as_str();
    let (accidental, rest) = if let Some(r) = rest.strip_prefix('#') { ("#", r) }
        else if let Some(r) = rest.strip_prefix('b') { ("b", r) }
        else { ("", rest) };
    let mode = match rest {
        "" => None,
        "m" | "min" | "minor" => Some("minor".to_owned()),
        "maj" | "major" => Some("major".to_owned()),
        _ => return None,
    };
    Some((format!("{}{}", root.to_ascii_uppercase(), accidental), mode, !accidental.is_empty() || !rest.is_empty()))
}

fn is_mode(word: &str) -> Option<&'static str> {
    match word.to_ascii_lowercase().as_str() {
        "major" | "maj" => Some("major"),
        "minor" | "min" => Some("minor"),
        _ => None,
    }
}

fn format_bpm(bpm: f64) -> String {
    let s = format!("{:.3}", bpm);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Validate only the four explicitly supported tokens, with {name} required.
/// Reject unknown braces/formatting and filename-unsafe literals before generating previews.
fn render_template(template: &str, name: &str, bpm: Option<f64>, key: Option<&str>, producer: Option<&str>) -> Result<String, String> {
    if template.trim().is_empty() || template.len() > 200 {
        return Err("Template must be between 1 and 200 bytes".into());
    }
    if template.chars().any(|ch| "<>:\"/\\|?*".contains(ch) || ch.is_control()) {
        return Err("Template contains Windows-unsafe characters".into());
    }
    let mut used = HashSet::new();
    let mut rendered = String::new();
    let mut remaining = template;
    while !remaining.is_empty() {
        if let Some(start) = remaining.find('{') {
            let before = &remaining[..start];
            if before.contains('}') { return Err("Invalid closing brace in template".into()); }
            rendered.push_str(before);
            let field_start = &remaining[start + 1..];
            let end = field_start.find('}').ok_or("Unclosed template placeholder")?;
            let token = &field_start[..end];
            if token.contains('{') { return Err("Nested placeholder".into()); }
            if !matches!(token, "name" | "bpm" | "key" | "producer") {
                return Err(format!("Unknown template field: {token}"));
            }
            if !used.insert(token) { return Err(format!("Duplicate template field: {token}")); }
            let value = match token {
                "name" => name.to_owned(),
                "bpm" => bpm.map(format_bpm).unwrap_or_default(),
                "key" => key.unwrap_or("").to_owned(),
                "producer" => producer.unwrap_or("").to_owned(),
                _ => unreachable!(),
            };
            rendered.push_str(&value);
            remaining = &field_start[end + 1..];
        } else {
            if remaining.contains('}') { return Err("Invalid closing brace in template".into()); }
            rendered.push_str(remaining);
            break;
        }
    }
    if !used.contains("name") { return Err("Template must contain {name}".into()); }
    // Remove redundant whitespace when optional metadata was missing.
    let cleaned = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() { return Err("Template produced an empty name".into()); }
    if cleaned.chars().count() > 240 { return Err("Filename would be too long".into()); }
    Ok(cleaned)
}

/// Returns a non-destructive suggestion. Existing BPM and key have priority over
/// FLP BPM / future key detection so a discrepancy can be reviewed manually.
pub fn preview_title(title: &str, flp_bpm: Option<f64>, detected_key: Option<&str>, template: &str) -> Result<NamePreview, String> {
    let title = title.trim();
    if title.is_empty() { return Err("Title cannot be empty".into()); }
    if title.chars().any(|ch| "<>:\"/\\|?*".contains(ch) || ch.is_control()) {
        return Err("Title contains Windows-unsafe characters".into());
    }
    let parts: Vec<&str> = title.split_whitespace().collect();
    if parse_bpm(parts[0]).is_some() &&
        (parts.len() == 1 || (parts.len() == 2 && parts[1].starts_with('@'))) {
        return Err("Title cannot consist of BPM metadata alone".into());
    }
    let mut excluded = HashSet::<usize>::new();
    let producer = parts.last().filter(|part| part.starts_with('@') && part.len() > 1)
        .map(|name| (*name).to_owned());
    if producer.is_some() { excluded.insert(parts.len() - 1); }

    // BPM must be a metadata token after the first title word, never the track name itself.
    let bpm_in_title = (1..parts.len()).rev().find_map(|i| {
        if excluded.contains(&i) { return None; }
        parse_bpm(parts[i]).map(|v| (i,v))
    });
    if let Some((i,_)) = bpm_in_title { excluded.insert(i); }

    let last_meta = (0..parts.len()).rev().find(|i| !excluded.contains(i));
    let mut key_in_title: Option<(String,Option<String>)> = None;
    let mut key_index: Option<usize> = None;
    let mut original_key_spelling: Option<String> = None;
    for i in (1..parts.len()).rev() {
        if excluded.contains(&i) { continue; }
        if let Some((canonical, mut mode, explicit)) = parse_key(parts[i]) {
            // A bare letter can also be an ordinary word. Only accept it in a
            // known metadata suffix (BPM or @producer). Accidentals are explicit.
            let trusted = explicit || (bpm_in_title.is_some() || (producer.is_some() && canonical != "A")) && Some(i) == last_meta;
            if !trusted { continue; }
            if mode.is_none() && i + 1 < parts.len() && !excluded.contains(&(i + 1)) {
                if let Some(extra_mode) = is_mode(parts[i+1]) {
                    mode = Some(extra_mode.into());
                    excluded.insert(i + 1);
                }
            }
            key_in_title = Some((canonical, mode));
            key_index = Some(i);
            original_key_spelling = Some(parts[i].chars().take(if parts[i].chars().nth(1).is_some_and(|c| c == '#' || c == 'b' || c == 'B') { 2 } else { 1 }).collect());
            break;
        }
        // Two-word modes: E minor, F# major. Verify the preceding note.
        if let Some(extra_mode) = is_mode(parts[i]) {
            if i > 1 && !excluded.contains(&(i-1)) {
                if let Some((canonical,_,_)) = parse_key(parts[i-1]) {
                    key_in_title = Some((canonical, Some(extra_mode.into())));
                    key_index = Some(i - 1);
                    original_key_spelling = Some(parts[i - 1].to_owned());
                    excluded.insert(i);
                    break;
                }
            }
        }
    }
    if let Some(i) = key_index { excluded.insert(i); }
    let name = parts.iter().enumerate().filter(|(i,_)| !excluded.contains(i)).map(|(_,p)| *p).collect::<Vec<_>>().join(" ");
    // If metadata parsing consumes the entire title, back off instead of
    // guessing a beat name. This also avoids turning `a 140` into an empty title.
    if name.is_empty() { return Err("Title has no beat name after metadata parsing".into()); }

    let title_bpm = bpm_in_title.map(|(_, bpm)| bpm);
    let bpm = title_bpm.or(flp_bpm.filter(|v| v.is_finite() && (10.0..=999.0).contains(v)));
    let conflict = title_bpm.zip(flp_bpm).is_some_and(|(a,b)| (a-b).abs() > 0.01);
    let parsed_key = if let Some((root,mode)) = key_in_title { Some((root,mode)) }
        else { detected_key.and_then(parse_key).map(|(root, mode, _)| (root,mode)) };
    let key = parsed_key.as_ref().map(|(root,_)| root.clone());
    let mode = parsed_key.as_ref().and_then(|(_,mode)| mode.clone());
    // Preserve a user's existing root-note spelling in the proposed filename.
    // The canonical root and mode remain available as structured metadata.
    let display_key = original_key_spelling.as_deref().or(key.as_deref());
    let proposed = render_template(template, &name, bpm, display_key, producer.as_deref())?;
    let missing_bpm = bpm.is_none();
    let missing_key = key.is_none();
    Ok(NamePreview {
        proposed, beat_name: name, bpm, key, mode, producer,
        missing_bpm, missing_key, bpm_conflict: conflict,
        needs_review: missing_bpm || missing_key || conflict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_user_examples_remain_unchanged() {
        for title in ["bigtruck 140 e @tofdidthat", "sup 140 f @tofdidthat", "spider 140 f# @tofdidthat"] {
            let result = preview_title(title,Some(140.0),None,DEFAULT_TEMPLATE).unwrap();
            assert_eq!(result.proposed, title.replace("  "," "));
            assert!(!result.needs_review);
        }
    }
    #[test]
    fn bpm_filled_without_inventing_key() {
        let result = preview_title("midnight @tofdidthat",Some(140.0),None,DEFAULT_TEMPLATE).unwrap();
        assert_eq!(result.proposed,"midnight 140 @tofdidthat");
        assert!(result.missing_key);
        assert!(!result.missing_bpm);
    }
    #[test]
    fn existing_bpm_never_silently_overwritten() {
        let result = preview_title("sup 140 f @tofdidthat",Some(142.0),None,DEFAULT_TEMPLATE).unwrap();
        assert_eq!(result.proposed,"sup 140 f @tofdidthat");
        assert!(result.bpm_conflict);
        assert!(result.needs_review);
    }
    #[test]
    fn detects_key_modes_and_manual_key() {
        let result = preview_title("night ride 128",Some(128.0),Some("F#m"),DEFAULT_TEMPLATE).unwrap();
        assert_eq!(result.proposed,"night ride 128 F#");
        assert_eq!(result.mode.as_deref(),Some("minor"));
        let result = preview_title("night ride 128 E minor @producer",Some(128.0),None,DEFAULT_TEMPLATE).unwrap();
        assert_eq!(result.key.as_deref(),Some("E"));
        assert_eq!(result.mode.as_deref(),Some("minor"));
    }
    #[test]
    fn no_false_bpm_from_808_and_no_false_key_from_word() {
        let result = preview_title("night 808 @producer",None,None,DEFAULT_TEMPLATE).unwrap();
        assert_eq!(result.beat_name,"night 808");
        assert!(result.missing_bpm);
        let result = preview_title("beat a melody",Some(140.0),None,DEFAULT_TEMPLATE).unwrap();
        assert!(result.missing_key);
    }
    #[test]
    fn templates_are_customizable_and_reject_unsafe_values() {
        let result = preview_title("coldheart 142 @producer",None,Some("E"),"{producer} - {name} - {bpm} - {key}").unwrap();
        assert_eq!(result.proposed,"@producer - coldheart - 142 - E");
        assert!(preview_title("beat",Some(120.0),None,"{name} {unknown}").is_err());
        assert!(preview_title("beat",Some(120.0),None,"{name}/{bpm}").is_err());
        assert!(preview_title("beat",Some(120.0),None,"{name}{name}").is_err());
    }
    #[test]
    fn refuses_names_with_no_title_or_unsafe_path_chars() {
        assert!(preview_title("140 @producer",Some(140.0),None,DEFAULT_TEMPLATE).is_err());
        assert!(preview_title("beat/escape 140",None,None,DEFAULT_TEMPLATE).is_err());
    }
}
