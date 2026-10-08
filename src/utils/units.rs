//! Eenhedenomrekenaar voor `!convert`: lengte, gewicht, inhoud, snelheid, oppervlakte, data, tijd, druk en temperatuur.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Length,
    Mass,
    Volume,
    Speed,
    Area,
    Data,
    Time,
    Pressure,
    Temperature,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Length => "lengte",
            Category::Mass => "gewicht",
            Category::Volume => "inhoud",
            Category::Speed => "snelheid",
            Category::Area => "oppervlakte",
            Category::Data => "data",
            Category::Time => "tijd",
            Category::Pressure => "druk",
            Category::Temperature => "temperatuur",
        }
    }
}

struct Unit {
    names: &'static [&'static str],
    cat: Category,
    /// Factor naar de basiseenheid van de categorie (m, kg, l, m/s, m², byte, s, Pa). Bij temperatuur onbenut.
    factor: f64,
}

const fn u(names: &'static [&'static str], cat: Category, factor: f64) -> Unit {
    Unit { names, cat, factor }
}

use Category::*;

const UNITS: &[Unit] = &[
    // lengte (m)
    u(&["m", "meter", "meters", "metre"], Length, 1.0),
    u(&["km", "kilometer", "kilometers"], Length, 1000.0),
    u(&["cm", "centimeter", "centimeters"], Length, 0.01),
    u(&["mm", "millimeter", "millimeters"], Length, 0.001),
    u(&["mi", "mile", "miles", "mijl", "mijlen"], Length, 1609.344),
    u(&["yd", "yard", "yards"], Length, 0.9144),
    u(&["ft", "feet", "foot", "voet", "voeten"], Length, 0.3048),
    u(&["in", "inch", "inches", "duim", "duimen"], Length, 0.0254),
    u(&["nmi", "zeemijl", "zeemijlen"], Length, 1852.0),
    // gewicht (kg)
    u(&["kg", "kilo", "kilos", "kilogram", "kilograms"], Mass, 1.0),
    u(&["g", "gram", "grams", "gr"], Mass, 0.001),
    u(&["mg", "milligram"], Mass, 0.000_001),
    u(&["t", "ton", "tonne", "tonnes"], Mass, 1000.0),
    u(&["lb", "lbs", "pound", "pounds", "pond"], Mass, 0.453_592_37),
    u(&["oz", "ounce", "ounces"], Mass, 0.028_349_523_125),
    u(&["ons"], Mass, 0.1),
    u(&["st", "stone"], Mass, 6.350_293_18),
    // inhoud (l)
    u(&["l", "liter", "liters", "litre"], Volume, 1.0),
    u(&["ml", "milliliter"], Volume, 0.001),
    u(&["cl", "centiliter"], Volume, 0.01),
    u(&["dl", "deciliter"], Volume, 0.1),
    u(&["gal", "gallon", "gallons"], Volume, 3.785_411_784),
    u(&["qt", "quart"], Volume, 0.946_352_946),
    u(&["pt", "pint", "pints"], Volume, 0.473_176_473),
    u(&["cup", "cups"], Volume, 0.236_588_236_5),
    u(&["floz", "fl.oz"], Volume, 0.029_573_529_562_5),
    u(&["tbsp", "eetlepel"], Volume, 0.014_786_764_781_25),
    u(&["tsp", "theelepel"], Volume, 0.004_928_921_593_75),
    // snelheid (m/s)
    u(&["m/s", "mps"], Speed, 1.0),
    u(&["km/h", "kmh", "kph", "kmu", "km/u"], Speed, 1.0 / 3.6),
    u(&["mph", "mi/h"], Speed, 0.447_04),
    u(&["kn", "kt", "knot", "knots", "knoop", "knopen"], Speed, 0.514_444_444),
    // oppervlakte (m²)
    u(&["m2", "m²", "sqm"], Area, 1.0),
    u(&["cm2", "cm²"], Area, 0.0001),
    u(&["km2", "km²"], Area, 1_000_000.0),
    u(&["ha", "hectare"], Area, 10_000.0),
    u(&["acre", "acres"], Area, 4_046.856_422_4),
    u(&["ft2", "ft²", "sqft"], Area, 0.092_903_04),
    // data (byte; SI = 1000, binair = 1024)
    u(&["bit", "bits"], Data, 0.125),
    u(&["byte", "bytes", "b"], Data, 1.0),
    u(&["kb", "kilobyte"], Data, 1e3),
    u(&["mb", "megabyte"], Data, 1e6),
    u(&["gb", "gigabyte"], Data, 1e9),
    u(&["tb", "terabyte"], Data, 1e12),
    u(&["kib"], Data, 1024.0),
    u(&["mib"], Data, 1_048_576.0),
    u(&["gib"], Data, 1_073_741_824.0),
    u(&["tib"], Data, 1_099_511_627_776.0),
    // tijd (s)
    u(&["s", "sec", "seconde", "seconden", "second", "seconds"], Time, 1.0),
    u(&["min", "minuut", "minuten", "minute", "minutes"], Time, 60.0),
    u(&["h", "hr", "uur", "uren", "hour", "hours"], Time, 3600.0),
    u(&["d", "dag", "dagen", "day", "days"], Time, 86_400.0),
    u(&["wk", "week", "weken", "weeks"], Time, 604_800.0),
    // druk (Pa)
    u(&["pa", "pascal"], Pressure, 1.0),
    u(&["hpa", "mbar"], Pressure, 100.0),
    u(&["kpa"], Pressure, 1000.0),
    u(&["bar"], Pressure, 100_000.0),
    u(&["psi"], Pressure, 6_894.757_293_168),
    u(&["atm"], Pressure, 101_325.0),
    u(&["mmhg", "torr"], Pressure, 133.322_387_415),
    // temperatuur
    u(&["c", "°c", "celsius"], Temperature, 1.0),
    u(&["f", "°f", "fahrenheit"], Temperature, 1.0),
    u(&["k", "kelvin"], Temperature, 1.0),
];

fn find(name: &str) -> Option<&'static Unit> {
    let n = name.trim().to_lowercase();
    UNITS.iter().find(|u| u.names.contains(&n.as_str()))
}

fn to_celsius(name: &str, v: f64) -> f64 {
    match name {
        "f" | "°f" | "fahrenheit" => (v - 32.0) * 5.0 / 9.0,
        "k" | "kelvin" => v - 273.15,
        _ => v,
    }
}

fn from_celsius(name: &str, c: f64) -> f64 {
    match name {
        "f" | "°f" | "fahrenheit" => c * 9.0 / 5.0 + 32.0,
        "k" | "kelvin" => c + 273.15,
        _ => c,
    }
}

pub fn convert(value: f64, from: &str, to: &str) -> Result<f64, String> {
    let a = find(from).ok_or_else(|| format!("onbekende eenheid '{}'", from))?;
    let b = find(to).ok_or_else(|| format!("onbekende eenheid '{}'", to))?;
    if a.cat != b.cat {
        return Err(format!("{} ({}) kan niet naar {} ({}) worden omgerekend", from, a.cat.name(), to, b.cat.name()));
    }
    if a.cat == Temperature {
        let (f, t) = (from.trim().to_lowercase(), to.trim().to_lowercase());
        let c = to_celsius(&f, value);
        if c < -273.15 - 1e-9 {
            return Err("onder het absolute nulpunt".into());
        }
        return Ok(from_celsius(&t, c));
    }
    Ok(value * a.factor / b.factor)
}

/// Splitst "5 km mi", "5km naar mi", "-40 c to f", "1,5 l ml" in (waarde, van, naar).
pub fn parse_request(args: &str) -> Result<(f64, String, String), String> {
    let s = args.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | 'e' | 'E'))).unwrap_or(s.len());
    // Een 'e' direct voor een eenheid ("5 eetlepel") hoort niet bij het getal
    let mut num_end = split;
    while num_end > 0 && matches!(s.as_bytes()[num_end - 1], b'e' | b'E' | b'-' | b'+') {
        num_end -= 1;
    }
    let (num, rest) = s.split_at(num_end);
    let value: f64 = num.trim().replace(',', ".").parse().map_err(|_| "geen geldig getal".to_string())?;
    if !value.is_finite() {
        return Err("geen geldig getal".into());
    }
    let mut toks: Vec<&str> = rest.split_whitespace().collect();
    // "5 km naar mi": het verbindingswoord in het midden valt weg (bij twee woorden blijft "in" een eenheid)
    if toks.len() == 3 && matches!(toks[1].to_lowercase().as_str(), "to" | "naar" | "in" | "=" | "->" | "→") {
        toks.remove(1);
    }
    // Eenheid direct tegen het getal ("5km") is al gesplitst; hier moeten er twee overblijven
    match toks.as_slice() {
        [from, to] => Ok((value, from.to_string(), to.to_string())),
        _ => Err("gebruik: !convert <getal> <van> <naar>, bijv. !convert 5 km mi".into()),
    }
}

/// Rondt af op `digits` significante cijfers (voor leesbare uitvoer).
pub fn round_sig(v: f64, digits: i32) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let scale = 10f64.powi(digits - 1 - v.abs().log10().floor() as i32);
    (v * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(args: &str) -> Result<String, String> {
        let (v, from, to) = parse_request(args)?;
        convert(v, &from, &to).map(|r| crate::utils::calc::format_number(round_sig(r, 6)))
    }

    #[test]
    fn common_conversions() {
        assert_eq!(conv("5 km mi").unwrap(), "3.10686");
        assert_eq!(conv("5km naar mi").unwrap(), "3.10686");
        assert_eq!(conv("5 km to mi").unwrap(), "3.10686");
        assert_eq!(conv("12 in cm").unwrap(), "30.48", "bij twee woorden is 'in' een eenheid");
        assert_eq!(conv("100 c f").unwrap(), "212");
        assert_eq!(conv("-40 c f").unwrap(), "-40");
        assert_eq!(conv("0 k c").unwrap(), "-273.15");
        assert_eq!(conv("72 °F °C").unwrap(), "22.2222");
        assert_eq!(conv("1,5 l ml").unwrap(), "1500");
        assert_eq!(conv("3 ons g").unwrap(), "300");
        assert_eq!(conv("1 gib mib").unwrap(), "1024");
        assert_eq!(conv("1 gb mb").unwrap(), "1000");
        assert_eq!(conv("100 km/h mph").unwrap(), "62.1371");
        assert_eq!(conv("2 eetlepel ml").unwrap(), "29.5735");
        assert_eq!(conv("1 atm hpa").unwrap(), "1013.25");
    }

    #[test]
    fn errors() {
        assert!(conv("5 km kg").unwrap_err().contains("kan niet naar"));
        assert!(conv("5 foo mi").unwrap_err().contains("onbekende eenheid 'foo'"));
        assert!(conv("km mi").unwrap_err().contains("geen geldig getal"));
        assert!(conv("5 km").unwrap_err().contains("gebruik"));
        assert!(conv("-300 c k").unwrap_err().contains("absolute nulpunt"));
        assert!(conv("").is_err());
    }

    #[test]
    fn rounding() {
        assert_eq!(round_sig(3.106_855_961_1, 6), 3.10686);
        assert_eq!(round_sig(0.0, 6), 0.0);
        assert_eq!(round_sig(123_456_789.0, 3), 123_000_000.0);
    }
}
