//! 物理键标识与键名表。
//!
//! KeyId = 扫描码低 8 位 | (扩展键 ? 0x100 : 0)。
//! 必须用扫描码而非 vkCode：左右 Ctrl/Alt 只靠 E0 扩展标志区分，
//! 且扫描码映射的是物理键位，不受键盘布局与输入法影响。

pub type KeyId = u16;

pub const MOD_CTRL: u8 = 0b0001;
pub const MOD_SHIFT: u8 = 0b0010;
pub const MOD_ALT: u8 = 0b0100;
pub const MOD_WIN: u8 = 0b1000;

pub const K_ESC: KeyId = 0x001;

/// 别名放在具体键名之前，显示时优先命中别名（Ctrl 比 LCtrl 更易读）。
const NAMES: &[(&str, KeyId)] = &[
    ("Ctrl", 0x01D), ("LCtrl", 0x01D), ("RCtrl", 0x11D),
    ("Shift", 0x02A), ("LShift", 0x02A), ("RShift", 0x036),
    ("Alt", 0x038), ("LAlt", 0x038), ("RAlt", 0x138),
    ("Win", 0x15B), ("LWin", 0x15B), ("RWin", 0x15C),
    ("Esc", 0x001), ("1", 0x002), ("2", 0x003), ("3", 0x004), ("4", 0x005),
    ("5", 0x006), ("6", 0x007), ("7", 0x008), ("8", 0x009), ("9", 0x00A), ("0", 0x00B),
    ("Minus", 0x00C), ("Equal", 0x00D), ("Backspace", 0x00E), ("Tab", 0x00F),
    ("Q", 0x010), ("W", 0x011), ("E", 0x012), ("R", 0x013), ("T", 0x014), ("Y", 0x015),
    ("U", 0x016), ("I", 0x017), ("O", 0x018), ("P", 0x019),
    ("LBracket", 0x01A), ("RBracket", 0x01B), ("Enter", 0x01C),
    ("A", 0x01E), ("S", 0x01F), ("D", 0x020), ("F", 0x021), ("G", 0x022), ("H", 0x023),
    ("J", 0x024), ("K", 0x025), ("L", 0x026),
    ("Semicolon", 0x027), ("Quote", 0x028), ("Backtick", 0x029), ("Backslash", 0x02B),
    ("Z", 0x02C), ("X", 0x02D), ("C", 0x02E), ("V", 0x02F), ("B", 0x030), ("N", 0x031),
    ("M", 0x032), ("Comma", 0x033), ("Period", 0x034), ("Slash", 0x035),
    ("NumMul", 0x037), ("Space", 0x039), ("CapsLock", 0x03A),
    ("F1", 0x03B), ("F2", 0x03C), ("F3", 0x03D), ("F4", 0x03E), ("F5", 0x03F),
    ("F6", 0x040), ("F7", 0x041), ("F8", 0x042), ("F9", 0x043), ("F10", 0x044),
    ("NumLock", 0x045), ("ScrollLock", 0x046),
    ("Num7", 0x047), ("Num8", 0x048), ("Num9", 0x049), ("NumMinus", 0x04A),
    ("Num4", 0x04B), ("Num5", 0x04C), ("Num6", 0x04D), ("NumPlus", 0x04E),
    ("Num1", 0x04F), ("Num2", 0x050), ("Num3", 0x051), ("Num0", 0x052), ("NumDot", 0x053),
    ("F11", 0x057), ("F12", 0x058),
    ("NumEnter", 0x11C), ("NumDiv", 0x135), ("PrintScreen", 0x137),
    ("Home", 0x147), ("Up", 0x148), ("PageUp", 0x149), ("Left", 0x14B),
    ("Right", 0x14D), ("End", 0x14F), ("Down", 0x150), ("PageDown", 0x151),
    ("Insert", 0x152), ("Delete", 0x153),
];

/// 返回该键所属的修饰键族。主键（非修饰键）返回 None。
pub fn mod_family(id: KeyId) -> Option<u8> {
    match id {
        0x01D | 0x11D => Some(MOD_CTRL),
        0x02A | 0x036 => Some(MOD_SHIFT),
        0x038 | 0x138 => Some(MOD_ALT),
        0x15B | 0x15C => Some(MOD_WIN),
        _ => None,
    }
}

pub fn is_modifier(id: KeyId) -> bool {
    mod_family(id).is_some()
}

/// 未知键回退为 SC 十六进制写法，保证捕获到的任意键都能无损存取。
pub fn key_name(id: KeyId) -> String {
    for (name, key) in NAMES {
        if *key == id {
            return (*name).to_string();
        }
    }
    format!("SC{id:03X}")
}

pub fn parse_key(s: &str) -> Option<KeyId> {
    let t = s.trim();
    for (name, key) in NAMES {
        if name.eq_ignore_ascii_case(t) {
            return Some(*key);
        }
    }
    t.strip_prefix("SC")
        .or_else(|| t.strip_prefix("sc"))
        .and_then(|hex| u16::from_str_radix(hex, 16).ok())
}

/// 从 (族掩码, 主键) 还原用于显示/保存的组合键序列。
/// 族掩码不区分左右，统一展开为左侧具体键。
pub fn combo_from_signature(mods: u8, trigger: KeyId) -> Vec<KeyId> {
    let mut v = Vec::new();
    if mods & MOD_CTRL != 0 { v.push(0x01D); }
    if mods & MOD_SHIFT != 0 { v.push(0x02A); }
    if mods & MOD_ALT != 0 { v.push(0x038); }
    if mods & MOD_WIN != 0 { v.push(0x15B); }
    v.push(trigger);
    v
}

/// 组合键签名：修饰键族掩码 + 主键。
pub fn mods_of(ids: &[KeyId]) -> u8 {
    let mut mods = 0;
    for id in ids {
        if let Some(m) = mod_family(*id) {
            mods |= m;
        }
    }
    mods
}

pub fn format_combo(ids: &[KeyId]) -> String {
    ids.iter()
        .map(|id| key_name(*id))
        .collect::<Vec<_>>()
        .join("+")
}

/// 解析 `Ctrl+J` 形式的组合。
/// 返回 (修饰键族掩码, 主键, 完整按键序列)。前置项必须是修饰键，末项必须不是。
pub fn parse_combo(s: &str) -> Result<(u8, KeyId, Vec<KeyId>), String> {
    let parts: Vec<&str> = s.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err("组合键为空".into());
    }
    let mut seq = Vec::with_capacity(parts.len());
    for part in &parts {
        match parse_key(part) {
            Some(id) => seq.push(id),
            None => return Err(format!("未知键名: {part}")),
        }
    }
    let (last, mods) = seq.split_last().unwrap();
    if is_modifier(*last) {
        return Err("组合键末项必须是主键（非 Ctrl/Shift/Alt/Win）".into());
    }
    if mods.iter().any(|id| !is_modifier(*id)) {
        return Err("只有修饰键可以放在主键之前".into());
    }
    Ok((mods_of(&seq), *last, seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_and_right_modifiers_are_distinct() {
        assert_eq!(parse_key("LCtrl"), Some(0x01D));
        assert_eq!(parse_key("RCtrl"), Some(0x11D));
        assert_eq!(parse_key("LAlt"), Some(0x038));
        assert_eq!(parse_key("RAlt"), Some(0x138));
        assert_eq!(mod_family(0x11D), Some(MOD_CTRL));
        assert_eq!(mod_family(0x138), Some(MOD_ALT));
        assert_eq!(mod_family(0x024), None);
    }

    #[test]
    fn combo_round_trip_keeps_signature() {
        for text in ["F5", "Ctrl+J", "Ctrl+Shift+K", "Alt+F4", "Win+Up", "LCtrl+RCtrl+K"] {
            let (mods, trigger, seq) = parse_combo(text).unwrap();
            let rebuilt = format_combo(&combo_from_signature(mods, trigger));
            let again = parse_combo(&rebuilt).unwrap();
            assert_eq!(again.0, mods, "{text}");
            assert_eq!(again.1, trigger, "{text}");
            assert_eq!(seq.last(), Some(&trigger), "{text}");
        }
    }

    #[test]
    fn unknown_key_uses_scancode_fallback() {
        assert_eq!(key_name(0x0FF), "SC0FF");
        assert_eq!(parse_key("SC0FF"), Some(0x0FF));
        assert_eq!(key_name(0x154), "SC154");
        assert_eq!(parse_key("sc154"), Some(0x154));
    }

    #[test]
    fn rejects_bad_combos() {
        assert!(parse_combo("").is_err());
        assert!(parse_combo("Ctrl").is_err());
        assert!(parse_combo("J+Ctrl").is_err());
        assert!(parse_combo("J+K").is_err());
        assert!(parse_combo("NoSuchKey").is_err());
    }

    /// README 里出现的示例必须能被真实解析，避免文档与实现脱节。
    #[test]
    fn documented_examples_parse() {
        for combo in ["F1", "Win+Shift+S", "PrintScreen", "Alt+PrintScreen", "Ctrl+J", "Ctrl+Shift+K", "Alt+F4"] {
            assert!(parse_combo(combo).is_ok(), "{combo}");
        }
    }
}
