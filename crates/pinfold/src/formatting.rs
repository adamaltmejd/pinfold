//! Control-character escaping for terminal diagnostics.

pub(crate) fn terminal_text(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        if ch.is_control() {
            escaped.extend(ch.escape_default());
        } else {
            escaped.push(ch);
        }
    }
    escaped
}
