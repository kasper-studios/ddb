/// Конвертирует имена клавиш в формат `System.Windows.Forms.SendKeys`
/// https://learn.microsoft.com/en-us/dotnet/api/system.windows.forms.sendkeys
pub fn normalize_windows_sendkeys(input: &str) -> String {
    let raw = input.trim();
    if raw.is_empty() {
        return String::new();
    }

    // Проверяем комбинации клавиш (например: "ctrl+c", "ctrl+alt+del", "ctrl+v")
    if raw.contains('+') {
        let parts: Vec<&str> = raw.split('+').map(|p| p.trim()).collect();
        let mut prefix = String::new();
        let mut key_part = "";

        for part in parts {
            let lower = part.to_lowercase();
            match lower.as_str() {
                "ctrl" | "control" => prefix.push('^'),
                "shift" => prefix.push('+'),
                "alt" => prefix.push('%'),
                "win" | "super" | "meta" => {
                    // SendKeys не поддерживает Windows Key напрямую, но переводим в эмуляцию Ctrl+Esc или спецсимвол
                    prefix.push_str("^{ESC}");
                }
                _ => key_part = part,
            }
        }

        let translated_key = translate_single_key(key_part);
        if !prefix.is_empty() && !translated_key.is_empty() {
            if translated_key.starts_with('{') && translated_key.ends_with('}') {
                return format!("{}{}", prefix, translated_key);
            } else {
                return format!("{}({})", prefix, translated_key);
            }
        }
    }

    translate_single_key(raw)
}

fn translate_single_key(key: &str) -> String {
    let lower = key.to_lowercase();
    match lower.as_str() {
        // Return / Enter
        "enter" | "return" | "cr" => "{ENTER}".to_string(),
        
        // Escape
        "esc" | "escape" => "{ESC}".to_string(),
        
        // Backspace / Delete
        "backspace" | "back" => "{BACKSPACE}".to_string(),
        "delete" | "del" => "{DELETE}".to_string(),
        
        // Tab / Space
        "tab" => "{TAB}".to_string(),
        "space" => " ".to_string(),
        
        // Arrows
        "up" | "arrowup" => "{UP}".to_string(),
        "down" | "arrowdown" => "{DOWN}".to_string(),
        "left" | "arrowleft" => "{LEFT}".to_string(),
        "right" | "arrowright" => "{RIGHT}".to_string(),
        
        // Navigation / Page
        "home" => "{HOME}".to_string(),
        "end" => "{END}".to_string(),
        "pageup" | "pgup" => "{PGUP}".to_string(),
        "pagedown" | "pgdn" => "{PGDN}".to_string(),
        
        // Modifiers (нажатие одиночных клавиш)
        "shift" => "+".to_string(),
        "ctrl" | "control" => "^".to_string(),
        "alt" => "%".to_string(),
        "super" | "win" | "meta" => "^{ESC}".to_string(),
        
        // Function keys F1-F12
        "f1" => "{F1}".to_string(),
        "f2" => "{F2}".to_string(),
        "f3" => "{F3}".to_string(),
        "f4" => "{F4}".to_string(),
        "f5" => "{F5}".to_string(),
        "f6" => "{F6}".to_string(),
        "f7" => "{F7}".to_string(),
        "f8" => "{F8}".to_string(),
        "f9" => "{F9}".to_string(),
        "f10" => "{F10}".to_string(),
        "f11" => "{F11}".to_string(),
        "f12" => "{F12}".to_string(),
        
        // Insert / PrtSc / Lock
        "insert" | "ins" => "{INSERT}".to_string(),
        "printscreen" | "prtsc" => "{PRTSC}".to_string(),
        "capslock" => "{CAPSLOCK}".to_string(),
        "numlock" => "{NUMLOCK}".to_string(),
        "scrolllock" => "{SCROLLLOCK}".to_string(),

        // Если это уже обёрнутый SendKeys код (например "{ENTER}")
        s if s.starts_with('{') && s.ends_with('}') => key.to_string(),

        // SendKeys спец-символы, требующие экранирования в фигурках
        "+" => "{+}".to_string(),
        "^" => "{^}".to_string(),
        "%" => "{%}".to_string(),
        "~" => "{~}".to_string(),
        "(" => "{(}".to_string(),
        ")" => "{)}".to_string(),
        "[" => "{[}".to_string(),
        "]" => "{]}".to_string(),
        "{" => "{{}".to_string(),
        "}" => "{}}".to_string(),

        // Обычный символ / текст
        _ => key.replace("'", "''"),
    }
}
