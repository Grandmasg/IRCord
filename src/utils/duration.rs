//! Tijdsduren zoals "90s", "10m", "1h30m", "2d" of "1u30m" (Nederlands "u" = uur) lezen en weergeven.

/// Leest een duur in seconden. Een kaal getal telt als `default_unit_secs` (bijv. 60 = minuten).
/// Geeft `None` bij ongeldige invoer, nul, of meer dan een jaar.
pub fn parse_duration(input: &str, default_unit_secs: i64) -> Option<i64> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    // Kaal getal
    if s.chars().all(|c| c.is_ascii_digit()) {
        let n: i64 = s.parse().ok()?;
        return finish(n.checked_mul(default_unit_secs)?);
    }
    let mut total: i64 = 0;
    let mut num = String::new();
    let mut seen_unit = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let mult = match c {
            's' => 1,
            'm' => 60,
            'h' | 'u' => 3600,
            'd' => 86_400,
            'w' => 604_800,
            ' ' => continue,
            _ => return None,
        };
        if num.is_empty() {
            return None;
        }
        let n: i64 = num.parse().ok()?;
        total = total.checked_add(n.checked_mul(mult)?)?;
        num.clear();
        seen_unit = true;
    }
    // Een getal zonder eenheid aan het eind ("1h30") is dubbelzinnig
    if !num.is_empty() || !seen_unit {
        return None;
    }
    finish(total)
}

fn finish(secs: i64) -> Option<i64> {
    (secs > 0 && secs <= 365 * 86_400).then_some(secs)
}

/// "1d 2u 3m 4s" (Nederlands) of "1d 2h 3m 4s"; nul-eenheden worden weggelaten.
pub fn format_duration(secs: i64, dutch: bool) -> String {
    let (d, h, m, s) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60, secs % 60);
    let hour = if dutch { 'u' } else { 'h' };
    let mut parts = Vec::new();
    if d > 0 { parts.push(format!("{}d", d)); }
    if h > 0 { parts.push(format!("{}{}", h, hour)); }
    if m > 0 { parts.push(format!("{}m", m)); }
    if s > 0 || parts.is_empty() { parts.push(format!("{}s", s)); }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units_and_compounds() {
        assert_eq!(parse_duration("90s", 60), Some(90));
        assert_eq!(parse_duration("10m", 60), Some(600));
        assert_eq!(parse_duration("1h30m", 60), Some(5400));
        assert_eq!(parse_duration("1u30m", 60), Some(5400));
        assert_eq!(parse_duration("2D", 60), Some(172_800));
        assert_eq!(parse_duration("1w", 60), Some(604_800));
        assert_eq!(parse_duration("1h 30m", 60), Some(5400));
        assert_eq!(parse_duration("45", 60), Some(2700), "kaal getal = minuten");
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        for bad in ["", "m", "h30", "1h30", "abc", "0m", "-5m", "5é", "1é", "99999999999999999999m", "400d", "1.5h", "５m"] {
            assert_eq!(parse_duration(bad, 60), None, "{bad:?}");
        }
    }

    #[test]
    fn formatting() {
        assert_eq!(format_duration(5400, true), "1u 30m");
        assert_eq!(format_duration(5400, false), "1h 30m");
        assert_eq!(format_duration(90_061, false), "1d 1h 1m 1s");
        assert_eq!(format_duration(45, true), "45s");
    }
}
