use std::process::Command;

pub(crate) fn user_locale() -> String {
    detect_user_locale().unwrap_or_else(|| {
        eprintln!("Warning: locale detection failed, falling back to English");
        String::new()
    })
}

fn detect_user_locale() -> Option<String> {
    env_locale()
        .or_else(platform_locale)
        .and_then(|value| normalize_locale(&value))
}

fn env_locale() -> Option<String> {
    const KEYS: [&str; 4] = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"];
    for key in KEYS {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        let candidate = if key == "LANGUAGE" {
            value.split(':').next().unwrap_or("")
        } else {
            value.as_str()
        };
        if !candidate.trim().is_empty() {
            return Some(candidate.to_owned());
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn platform_locale() -> Option<String> {
    let output = Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[cfg(windows)]
fn platform_locale() -> Option<String> {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    let mut buffer = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if len <= 1 {
        return None;
    }
    String::from_utf16(&buffer[..(len as usize - 1)]).ok()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_locale() -> Option<String> {
    None
}

fn normalize_locale(input: &str) -> Option<String> {
    let mut locale = input.trim();
    if locale.is_empty() || locale.eq_ignore_ascii_case("c") || locale.eq_ignore_ascii_case("posix")
    {
        return None;
    }

    if let Some((head, _)) = locale.split_once('.') {
        locale = head;
    }
    if let Some((head, _)) = locale.split_once('@') {
        locale = head;
    }

    let mut parts = locale
        .replace('_', "-")
        .split('-')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return None;
    }

    parts[0] = parts[0].to_ascii_lowercase();
    for part in parts.iter_mut().skip(1) {
        if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            let first = chars.next().unwrap().to_ascii_uppercase();
            let rest = chars.as_str().to_ascii_lowercase();
            *part = format!("{first}{rest}");
            continue;
        }
        if (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
            || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
        {
            *part = part.to_ascii_uppercase();
        }
    }

    Some(parts.join("-"))
}

#[cfg(test)]
mod tests {
    use super::normalize_locale;

    #[test]
    fn normalizes_common_locale_forms() {
        assert_eq!(normalize_locale("en_US.UTF-8"), Some("en-US".to_owned()));
        assert_eq!(normalize_locale("de_AT@euro"), Some("de-AT".to_owned()));
        assert_eq!(
            normalize_locale("zh-hans-cn"),
            Some("zh-Hans-CN".to_owned())
        );
        assert_eq!(normalize_locale("es-419"), Some("es-419".to_owned()));
    }

    #[test]
    fn rejects_empty_and_posix_locales() {
        assert_eq!(normalize_locale(""), None);
        assert_eq!(normalize_locale("C"), None);
        assert_eq!(normalize_locale("POSIX"), None);
    }
}
