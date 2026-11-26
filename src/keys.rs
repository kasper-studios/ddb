/// Native Windows keyboard input via Win32 `SendInput` (`KEYBDINPUT` + `KEYEVENTF_UNICODE`)
/// and key-combo parser supporting names like `"ctrl+c"`, `"win+r"`, `"enter"`, `"{F4}"`, etc.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinKeySpec {
    pub vk: u16,
    pub extended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyTarget {
    Virtual(WinKeySpec),
    Unicode(char),
    CharWithModifiers(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKeyCombo {
    pub modifiers: Vec<WinKeySpec>,
    pub target: KeyTarget,
}

/// Parses a human/agent key specification (e.g. `"ctrl+c"`, `"win+r"`, `"enter"`, `"{PGUP}"`, `"+"`)
/// into Win32 virtual key codes and modifier states.
pub fn parse_key_combo(input: &str) -> anyhow::Result<ParsedKeyCombo> {
    if input == " " {
        return Ok(ParsedKeyCombo {
            modifiers: Vec::new(),
            target: KeyTarget::Virtual(WinKeySpec {
                vk: 0x20, // VK_SPACE
                extended: false,
            }),
        });
    }

    let raw = input.trim();
    if raw.is_empty() {
        return Err(anyhow::anyhow!("Missing or empty key"));
    }

    let parts = split_combo_parts(raw);
    let (mod_parts, key_part) = parts.split_at(parts.len() - 1);
    let key_token = key_part[0];

    let mut modifiers = Vec::with_capacity(mod_parts.len());
    for m in mod_parts {
        let spec = parse_modifier_token(m)
            .ok_or_else(|| anyhow::anyhow!("Unknown modifier '{}' in key '{}'", m, input))?;
        modifiers.push(spec);
    }

    let unwrapped = unwrap_sendkeys_braces(key_token);

    if let Some(spec) = parse_named_key(unwrapped) {
        return Ok(ParsedKeyCombo {
            modifiers,
            target: KeyTarget::Virtual(spec),
        });
    }

    let mut chars = unwrapped.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        if ch == ' ' {
            return Ok(ParsedKeyCombo {
                modifiers,
                target: KeyTarget::Virtual(WinKeySpec {
                    vk: 0x20, // VK_SPACE
                    extended: false,
                }),
            });
        }
        if ch.is_ascii_alphabetic() {
            let vk = ch.to_ascii_uppercase() as u16;
            if modifiers.is_empty() && ch.is_ascii_uppercase() {
                modifiers.push(WinKeySpec {
                    vk: 0x10, // VK_SHIFT
                    extended: false,
                });
            }
            return Ok(ParsedKeyCombo {
                modifiers,
                target: KeyTarget::Virtual(WinKeySpec {
                    vk,
                    extended: false,
                }),
            });
        }
        if ch.is_ascii_digit() {
            return Ok(ParsedKeyCombo {
                modifiers,
                target: KeyTarget::Virtual(WinKeySpec {
                    vk: ch as u16,
                    extended: false,
                }),
            });
        }

        let target = if modifiers.is_empty() {
            KeyTarget::Unicode(ch)
        } else {
            KeyTarget::CharWithModifiers(ch)
        };
        return Ok(ParsedKeyCombo { modifiers, target });
    }

    Err(anyhow::anyhow!("Unsupported key name: '{}'", input))
}

fn split_combo_parts(raw: &str) -> Vec<&str> {
    if raw == "+" || raw == "{+}" {
        return vec![raw];
    }

    let mut parts = Vec::new();
    let bytes = raw.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_braces = false;

    while i < bytes.len() {
        match bytes[i] {
            b'{' => in_braces = true,
            b'}' => in_braces = false,
            b'+' if !in_braces => {
                let seg = raw[start..i].trim();
                if !seg.is_empty() {
                    parts.push(seg);
                }
                start = i + 1;
                // Handle trailing '+' as the key token (e.g. "ctrl++").
                if start == bytes.len() - 1 && bytes[start] == b'+' {
                    parts.push(&raw[start..]);
                    return parts;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let tail = raw[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }

    if parts.is_empty() {
        parts.push(raw);
    }
    parts
}

fn unwrap_sendkeys_braces(token: &str) -> &str {
    match token {
        "{+}" => "+",
        "{^}" => "^",
        "{%}" => "%",
        "{~}" => "~",
        "{(}" => "(",
        "{)}" => ")",
        "{[}" => "[",
        "{]}" => "]",
        "{{}" => "{",
        "{}}" => "}",
        s if s.starts_with('{') && s.ends_with('}') && s.len() >= 3 => &s[1..s.len() - 1],
        s => s,
    }
}

fn parse_modifier_token(token: &str) -> Option<WinKeySpec> {
    match token.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(WinKeySpec { vk: 0x11, extended: false }), // VK_CONTROL
        "lctrl" | "lcontrol" => Some(WinKeySpec { vk: 0xA2, extended: false }), // VK_LCONTROL
        "rctrl" | "rcontrol" => Some(WinKeySpec { vk: 0xA3, extended: true }), // VK_RCONTROL
        "shift" => Some(WinKeySpec { vk: 0x10, extended: false }), // VK_SHIFT
        "lshift" => Some(WinKeySpec { vk: 0xA0, extended: false }), // VK_LSHIFT
        "rshift" => Some(WinKeySpec { vk: 0xA1, extended: false }), // VK_RSHIFT
        "alt" | "menu" => Some(WinKeySpec { vk: 0x12, extended: false }), // VK_MENU
        "lalt" | "lmenu" => Some(WinKeySpec { vk: 0xA4, extended: false }), // VK_LMENU
        "ralt" | "rmenu" | "altgr" => Some(WinKeySpec { vk: 0xA5, extended: true }), // VK_RMENU
        "win" | "super" | "meta" | "cmd" | "lwin" => {
            Some(WinKeySpec { vk: 0x5B, extended: true }) // VK_LWIN
        }
        "rwin" => Some(WinKeySpec { vk: 0x5C, extended: true }), // VK_RWIN
        _ => None,
    }
}

fn parse_named_key(token: &str) -> Option<WinKeySpec> {
    let lower = token.trim().to_ascii_lowercase();
    if let Some(mod_spec) = parse_modifier_token(&lower) {
        return Some(mod_spec);
    }

    match lower.as_str() {
        "enter" | "return" | "cr" => Some(WinKeySpec { vk: 0x0D, extended: false }),
        "esc" | "escape" => Some(WinKeySpec { vk: 0x1B, extended: false }),
        "backspace" | "back" | "bs" => Some(WinKeySpec { vk: 0x08, extended: false }),
        "delete" | "del" => Some(WinKeySpec { vk: 0x2E, extended: true }),
        "tab" => Some(WinKeySpec { vk: 0x09, extended: false }),
        "space" | "spacebar" => Some(WinKeySpec { vk: 0x20, extended: false }),

        "up" | "arrowup" => Some(WinKeySpec { vk: 0x26, extended: true }),
        "down" | "arrowdown" => Some(WinKeySpec { vk: 0x28, extended: true }),
        "left" | "arrowleft" => Some(WinKeySpec { vk: 0x25, extended: true }),
        "right" | "arrowright" => Some(WinKeySpec { vk: 0x27, extended: true }),

        "home" => Some(WinKeySpec { vk: 0x24, extended: true }),
        "end" => Some(WinKeySpec { vk: 0x23, extended: true }),
        "pageup" | "pgup" | "prior" => Some(WinKeySpec { vk: 0x21, extended: true }),
        "pagedown" | "pgdn" | "next" => Some(WinKeySpec { vk: 0x22, extended: true }),

        "insert" | "ins" => Some(WinKeySpec { vk: 0x2D, extended: true }),
        "printscreen" | "prtsc" | "prtscn" | "snapshot" => {
            Some(WinKeySpec { vk: 0x2C, extended: true })
        }
        "capslock" | "caps" => Some(WinKeySpec { vk: 0x14, extended: false }),
        "numlock" => Some(WinKeySpec { vk: 0x90, extended: true }),
        "scrolllock" | "scrlk" => Some(WinKeySpec { vk: 0x91, extended: false }),
        "pause" | "break" => Some(WinKeySpec { vk: 0x13, extended: false }),
        "apps" | "contextmenu" => Some(WinKeySpec { vk: 0x5D, extended: true }),

        "volumeup" | "volup" => Some(WinKeySpec { vk: 0xAF, extended: true }),
        "volumedown" | "voldown" => Some(WinKeySpec { vk: 0xAE, extended: true }),
        "volumemute" | "mute" => Some(WinKeySpec { vk: 0xAD, extended: true }),
        "mediaplaypause" | "playpause" => Some(WinKeySpec { vk: 0xB3, extended: true }),
        "medianext" | "nexttrack" => Some(WinKeySpec { vk: 0xB0, extended: true }),
        "mediaprev" | "prevtrack" => Some(WinKeySpec { vk: 0xB1, extended: true }),
        "mediastop" => Some(WinKeySpec { vk: 0xB2, extended: true }),

        "multiply" => Some(WinKeySpec { vk: 0x6A, extended: false }),
        "add" => Some(WinKeySpec { vk: 0x6B, extended: false }),
        "subtract" => Some(WinKeySpec { vk: 0x6D, extended: false }),
        "decimal" => Some(WinKeySpec { vk: 0x6E, extended: false }),
        "divide" => Some(WinKeySpec { vk: 0x6F, extended: true }),

        _ => {
            if let Some(num) = lower.strip_prefix('f').and_then(|s| s.parse::<u16>().ok()) {
                if (1..=24).contains(&num) {
                    return Some(WinKeySpec {
                        vk: 0x70 + (num - 1),
                        extended: false,
                    });
                }
            }
            if let Some(num) = lower.strip_prefix("numpad").and_then(|s| s.parse::<u16>().ok()) {
                if num <= 9 {
                    return Some(WinKeySpec {
                        vk: 0x60 + num,
                        extended: false,
                    });
                }
            }
            None
        }
    }
}

#[cfg(target_os = "windows")]
fn vk_input(spec: WinKeySpec, key_up: bool) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    };

    let scan = unsafe { MapVirtualKeyW(spec.vk as u32, MAPVK_VK_TO_VSC) } as u16;
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if spec.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(spec.vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(target_os = "windows")]
fn unicode_inputs(ch: char, out: &mut Vec<windows::Win32::UI::Input::KeyboardAndMouse::INPUT>) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
    };

    let mut buf = [0u16; 2];
    let units = ch.encode_utf16(&mut buf);
    for &unit in units.iter() {
        out.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }
    for &unit in units.iter().rev() {
        out.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }
}

#[cfg(target_os = "windows")]
fn dispatch_inputs(inputs: &[windows::Win32::UI::Input::KeyboardAndMouse::INPUT]) -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

    if inputs.is_empty() {
        return Ok(());
    }

    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(anyhow::anyhow!(
            "Win32 SendInput sent {}/{} keyboard events (last error: {})",
            sent,
            inputs.len(),
            windows::core::Error::from_thread()
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn resolve_char_vk_with_fallback(ch: char) -> Option<(WinKeySpec, bool)> {
    use windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;

    let mut buf = [0u16; 2];
    let units = ch.encode_utf16(&mut buf);
    if units.len() == 1 {
        let scan = unsafe { VkKeyScanW(units[0]) };
        if scan != -1 {
            let vk = (scan & 0xFF) as u16;
            let shift_needed = ((scan >> 8) & 1) != 0;
            return Some((WinKeySpec { vk, extended: false }, shift_needed));
        }
    }

    // Fallback to standard US-QWERTY OEM VK codes so shortcuts like "ctrl+-" or "ctrl+/"
    // still work even when a non-Latin keyboard layout (e.g. Russian) is active.
    let (vk, shift) = match ch {
        '-' => (0xBD, false),
        '_' => (0xBD, true),
        '=' => (0xBB, false),
        '+' => (0xBB, true),
        ',' => (0xBC, false),
        '<' => (0xBC, true),
        '.' => (0xBE, false),
        '>' => (0xBE, true),
        '/' => (0xBF, false),
        '?' => (0xBF, true),
        ';' => (0xBA, false),
        ':' => (0xBA, true),
        '\'' => (0xDE, false),
        '"' => (0xDE, true),
        '[' => (0xDB, false),
        '{' => (0xDB, true),
        ']' => (0xDD, false),
        '}' => (0xDD, true),
        '\\' => (0xDC, false),
        '|' => (0xDC, true),
        '`' => (0xC0, false),
        '~' => (0xC0, true),
        _ => return None,
    };
    Some((WinKeySpec { vk, extended: false }, shift))
}

/// Presses and releases a single key or key combination (e.g. `"ctrl+c"`, `"enter"`, `"win+r"`)
/// directly through Win32 `SendInput`.
#[cfg(target_os = "windows")]
pub fn send_key(input: &str) -> anyhow::Result<()> {
    let parsed = parse_key_combo(input)?;
    let mut events = Vec::with_capacity(parsed.modifiers.len() * 2 + 4);
    let mut active_mods = parsed.modifiers;

    match parsed.target {
        KeyTarget::Virtual(spec) => {
            for &m in &active_mods {
                events.push(vk_input(m, false));
            }
            events.push(vk_input(spec, false));
            events.push(vk_input(spec, true));
            for &m in active_mods.iter().rev() {
                events.push(vk_input(m, true));
            }
        }
        KeyTarget::Unicode(ch) => {
            unicode_inputs(ch, &mut events);
        }
        KeyTarget::CharWithModifiers(ch) => {
            let (spec, needs_shift) = resolve_char_vk_with_fallback(ch).ok_or_else(|| {
                anyhow::anyhow!("Character '{}' cannot be mapped for key combination '{}'", ch, input)
            })?;
            let has_shift = active_mods.iter().any(|m| matches!(m.vk, 0x10 | 0xA0 | 0xA1));
            if needs_shift && !has_shift {
                active_mods.push(WinKeySpec { vk: 0x10, extended: false });
            }
            for &m in &active_mods {
                events.push(vk_input(m, false));
            }
            events.push(vk_input(spec, false));
            events.push(vk_input(spec, true));
            for &m in active_mods.iter().rev() {
                events.push(vk_input(m, true));
            }
        }
    }

    dispatch_inputs(&events)
}

/// Types arbitrary Unicode text directly through Win32 `SendInput` (`KEYEVENTF_UNICODE`),
/// completely independent of the active Windows keyboard layout.
#[cfg(target_os = "windows")]
pub fn send_text(text: &str) -> anyhow::Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    let mut events = Vec::with_capacity(text.len() * 2);
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                let enter = WinKeySpec { vk: 0x0D, extended: false };
                events.push(vk_input(enter, false));
                events.push(vk_input(enter, true));
            }
            '\n' => {
                let enter = WinKeySpec { vk: 0x0D, extended: false };
                events.push(vk_input(enter, false));
                events.push(vk_input(enter, true));
            }
            '\t' => {
                let tab = WinKeySpec { vk: 0x09, extended: false };
                events.push(vk_input(tab, false));
                events.push(vk_input(tab, true));
            }
            other => {
                unicode_inputs(other, &mut events);
            }
        }
    }

    dispatch_inputs(&events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combos_and_case_insensitive_ctrl_c() {
        let parsed = parse_key_combo("Ctrl+C").unwrap();
        assert_eq!(
            parsed.modifiers,
            vec![WinKeySpec { vk: 0x11, extended: false }]
        );
        assert_eq!(
            parsed.target,
            KeyTarget::Virtual(WinKeySpec { vk: b'C' as u16, extended: false })
        );
    }

    #[test]
    fn parses_uppercase_single_letter_with_shift() {
        let parsed = parse_key_combo("A").unwrap();
        assert_eq!(
            parsed.modifiers,
            vec![WinKeySpec { vk: 0x10, extended: false }]
        );
        assert_eq!(
            parsed.target,
            KeyTarget::Virtual(WinKeySpec { vk: b'A' as u16, extended: false })
        );
    }

    #[test]
    fn parses_win_and_sendkeys_braces() {
        let win_r = parse_key_combo("win+r").unwrap();
        assert_eq!(
            win_r.modifiers,
            vec![WinKeySpec { vk: 0x5B, extended: true }]
        );
        assert_eq!(
            win_r.target,
            KeyTarget::Virtual(WinKeySpec { vk: b'R' as u16, extended: false })
        );

        let enter = parse_key_combo("{ENTER}").unwrap();
        assert!(enter.modifiers.is_empty());
        assert_eq!(
            enter.target,
            KeyTarget::Virtual(WinKeySpec { vk: 0x0D, extended: false })
        );
    }

    #[test]
    fn parses_plus_and_ctrl_plus() {
        let plus = parse_key_combo("+").unwrap();
        assert!(plus.modifiers.is_empty());
        assert_eq!(plus.target, KeyTarget::Unicode('+'));

        let ctrl_plus = parse_key_combo("ctrl++").unwrap();
        assert_eq!(
            ctrl_plus.modifiers,
            vec![WinKeySpec { vk: 0x11, extended: false }]
        );
        assert_eq!(ctrl_plus.target, KeyTarget::CharWithModifiers('+'));
    }

    #[test]
    fn parses_cyrillic_single_char_as_unicode() {
        let parsed = parse_key_combo("д").unwrap();
        assert!(parsed.modifiers.is_empty());
        assert_eq!(parsed.target, KeyTarget::Unicode('д'));
    }
}
