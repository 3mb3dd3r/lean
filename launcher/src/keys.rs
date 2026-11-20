//! LWJGL 2 `org.lwjgl.input.Keyboard` key names, plus `NONE` for unbound.

pub const KEY_NAMES: &[&str] = &[
    "NONE", "ESCAPE", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "MINUS", "EQUALS", "BACK",
    "TAB", "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "LBRACKET", "RBRACKET", "RETURN",
    "LCONTROL", "A", "S", "D", "F", "G", "H", "J", "K", "L", "SEMICOLON", "APOSTROPHE", "GRAVE",
    "LSHIFT", "BACKSLASH", "Z", "X", "C", "V", "B", "N", "M", "COMMA", "PERIOD", "SLASH", "RSHIFT",
    "MULTIPLY", "LMENU", "SPACE", "CAPITAL", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9",
    "F10", "NUMLOCK", "SCROLL", "NUMPAD7", "NUMPAD8", "NUMPAD9", "SUBTRACT", "NUMPAD4", "NUMPAD5",
    "NUMPAD6", "ADD", "NUMPAD1", "NUMPAD2", "NUMPAD3", "NUMPAD0", "DECIMAL", "F11", "F12", "F13",
    "F14", "F15", "F16", "F17", "F18", "KANA", "F19", "CONVERT", "NOCONVERT", "YEN", "NUMPADEQUALS",
    "CIRCUMFLEX", "AT", "COLON", "UNDERLINE", "KANJI", "STOP", "AX", "UNLABELED", "NUMPADENTER",
    "RCONTROL", "SECTION", "NUMPADCOMMA", "DIVIDE", "SYSRQ", "RMENU", "FUNCTION", "PAUSE", "HOME",
    "UP", "PRIOR", "LEFT", "RIGHT", "END", "DOWN", "NEXT", "INSERT", "DELETE", "CLEAR", "LMETA",
    "LWIN", "RMETA", "RWIN", "APPS", "POWER", "SLEEP",
];

/// Returns the canonical (upper-case) key name when `input` is a valid LWJGL 2 name.
pub fn normalize(input: &str) -> Option<String> {
    let up = input.trim().to_ascii_uppercase();
    KEY_NAMES.contains(&up.as_str()).then_some(up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_key_names() {
        assert_eq!(normalize("b").as_deref(), Some("B"));
        assert_eq!(normalize(" rshift ").as_deref(), Some("RSHIFT"));
        assert_eq!(normalize("NONE").as_deref(), Some("NONE"));
        assert_eq!(normalize("F12").as_deref(), Some("F12"));
        assert_eq!(normalize("7").as_deref(), Some("7"));
        assert_eq!(normalize("SHIFT"), None);
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("F20"), None);
        assert_eq!(normalize("CTRL"), None);
    }
}
