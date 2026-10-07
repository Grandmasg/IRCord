//! Veilige rekenmachine voor `!calc`: een eigen recursive-descent parser, geen eval.
//! Ondersteunt + - * / % ^, haakjes, constanten (pi, e) en de functies sqrt, cbrt, abs, sin, cos, tan, asin, acos,
//! atan, ln, log (10), log2, exp, round, floor, ceil, deg, rad. Komma en punt zijn beide decimaalteken.

const MAX_LEN: usize = 200;
const MAX_DEPTH: usize = 40;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
}

#[derive(Debug, PartialEq)]
pub enum CalcError {
    Empty,
    TooLong,
    Syntax(String),
    UnknownName(String),
    DivisionByZero,
    OutOfRange,
    TooDeep,
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalcError::Empty => write!(f, "geen som opgegeven"),
            CalcError::TooLong => write!(f, "de som is te lang (max. {} tekens)", MAX_LEN),
            CalcError::Syntax(m) => write!(f, "ongeldige som: {}", m),
            CalcError::UnknownName(n) => write!(f, "onbekende naam '{}'", n),
            CalcError::DivisionByZero => write!(f, "delen door nul"),
            CalcError::OutOfRange => write!(f, "uitkomst onbepaald of te groot"),
            CalcError::TooDeep => write!(f, "te veel geneste haakjes"),
        }
    }
}

fn tokenize(input: &str) -> Result<Vec<Tok>, CalcError> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut toks = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '+' => { toks.push(Tok::Plus); i += 1 }
            '-' | '−' => { toks.push(Tok::Minus); i += 1 }
            '*' | '×' | 'x' | '·' => { toks.push(Tok::Star); i += 1 }
            '/' | '÷' | ':' => { toks.push(Tok::Slash); i += 1 }
            '%' => { toks.push(Tok::Percent); i += 1 }
            '^' => { toks.push(Tok::Caret); i += 1 }
            '(' => { toks.push(Tok::LParen); i += 1 }
            ')' => { toks.push(Tok::RParen); i += 1 }
            c if c.is_ascii_digit() || c == '.' || c == ',' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',') {
                    i += 1;
                }
                // wetenschappelijke notatie: 1e3, 2.5e-4 (alleen als er een cijfer volgt)
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        while j < chars.len() && chars[j].is_ascii_digit() {
                            j += 1;
                        }
                        i = j;
                    }
                }
                let text: String = chars[start..i].iter().collect::<String>().replace(',', ".");
                let n: f64 = text.parse().map_err(|_| CalcError::Syntax(format!("'{}' is geen getal", text)))?;
                toks.push(Tok::Num(n));
            }
            c if c.is_alphabetic() || c == 'π' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == 'π') {
                    i += 1;
                }
                toks.push(Tok::Ident(chars[start..i].iter().collect::<String>().to_lowercase()));
            }
            other => return Err(CalcError::Syntax(format!("onverwacht teken '{}'", other))),
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn expr(&mut self) -> Result<f64, CalcError> {
        let mut v = self.term()?;
        while let Some(t) = self.peek() {
            match t {
                Tok::Plus => { self.pos += 1; v += self.term()?; }
                Tok::Minus => { self.pos += 1; v -= self.term()?; }
                _ => break,
            }
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, CalcError> {
        let mut v = self.unary()?;
        while let Some(t) = self.peek() {
            match t {
                Tok::Star => { self.pos += 1; v *= self.unary()?; }
                Tok::Slash => {
                    self.pos += 1;
                    let d = self.unary()?;
                    if d == 0.0 { return Err(CalcError::DivisionByZero); }
                    v /= d;
                }
                Tok::Percent => {
                    self.pos += 1;
                    let d = self.unary()?;
                    if d == 0.0 { return Err(CalcError::DivisionByZero); }
                    v %= d;
                }
                // impliciet vermenigvuldigen: 2(3+4), 2pi
                Tok::LParen | Tok::Ident(_) | Tok::Num(_) => { v *= self.unary()?; }
                _ => break,
            }
        }
        Ok(v)
    }

    // -2^2 = -(2^2), zoals in de wiskunde
    fn unary(&mut self) -> Result<f64, CalcError> {
        match self.peek() {
            Some(Tok::Minus) => { self.pos += 1; Ok(-self.unary()?) }
            Some(Tok::Plus) => { self.pos += 1; self.unary() }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<f64, CalcError> {
        let base = self.atom()?;
        if let Some(Tok::Caret) = self.peek() {
            self.pos += 1;
            let exp = self.unary()?; // rechts-associatief: 2^3^2 = 2^(3^2)
            let r = base.powf(exp);
            if !r.is_finite() { return Err(CalcError::OutOfRange); }
            return Ok(r);
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<f64, CalcError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(CalcError::TooDeep);
        }
        let r = match self.next() {
            Some(Tok::Num(n)) => Ok(n),
            Some(Tok::LParen) => {
                let v = self.expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err(CalcError::Syntax("ontbrekend sluithaakje".into())),
                }
            }
            Some(Tok::Ident(name)) => match name.as_str() {
                "pi" | "π" => Ok(std::f64::consts::PI),
                "e" => Ok(std::f64::consts::E),
                _ => {
                    if !matches!(self.peek(), Some(Tok::LParen)) {
                        return Err(CalcError::UnknownName(name));
                    }
                    self.pos += 1;
                    let arg = self.expr()?;
                    if !matches!(self.next(), Some(Tok::RParen)) {
                        return Err(CalcError::Syntax("ontbrekend sluithaakje".into()));
                    }
                    apply_fn(&name, arg)
                }
            },
            Some(other) => Err(CalcError::Syntax(format!("onverwacht {:?}", other))),
            None => Err(CalcError::Syntax("de som is onvolledig".into())),
        };
        self.depth -= 1;
        r
    }
}

fn apply_fn(name: &str, x: f64) -> Result<f64, CalcError> {
    let v = match name {
        "sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "abs" => x.abs(),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" => x.asin(),
        "acos" => x.acos(),
        "atan" => x.atan(),
        "ln" => x.ln(),
        "log" | "log10" => x.log10(),
        "log2" => x.log2(),
        "exp" => x.exp(),
        "round" => x.round(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "deg" => x.to_degrees(),
        "rad" => x.to_radians(),
        other => return Err(CalcError::UnknownName(other.to_string())),
    };
    if v.is_finite() { Ok(v) } else { Err(CalcError::OutOfRange) }
}

pub fn evaluate(input: &str) -> Result<f64, CalcError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(CalcError::Empty);
    }
    if input.chars().count() > MAX_LEN {
        return Err(CalcError::TooLong);
    }
    let toks = tokenize(input)?;
    let mut p = Parser { toks, pos: 0, depth: 0 };
    let v = p.expr()?;
    if p.pos < p.toks.len() {
        return Err(CalcError::Syntax("overbodige tekens aan het eind".into()));
    }
    if v.is_finite() { Ok(v) } else { Err(CalcError::OutOfRange) }
}

/// Mooie weergave: gehele getallen zonder komma, anders maximaal 10 significante cijfers; zeer groot/klein als 1.5e20.
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    if v.abs() >= 1e15 || v.abs() < 1e-6 {
        let sci = format!("{:.9e}", v);
        let (mantissa, exponent) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{}e{}", mantissa, exponent);
    }
    let s = format!("{:.10}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(s: &str) -> String {
        match evaluate(s) {
            Ok(v) => format_number(v),
            Err(e) => format!("ERR: {e}"),
        }
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(calc("2+2*3"), "8");
        assert_eq!(calc("(2+2)*3"), "12");
        assert_eq!(calc("2^3^2"), "512");
        assert_eq!(calc("-2^2"), "-4");
        assert_eq!(calc("10/4"), "2.5");
        assert_eq!(calc("7%3"), "1");
        assert_eq!(calc("1,5*2"), "3");
        assert_eq!(calc("1e3+1"), "1001");
        assert_eq!(calc("2(3+4)"), "14");
        assert_eq!(calc("2*-3"), "-6");
        assert_eq!(calc("0.1+0.2"), "0.3");
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(calc("sqrt(16)"), "4");
        assert_eq!(calc("sin(pi/2)"), "1");
        assert_eq!(calc("round(e*100)/100"), "2.72");
        assert_eq!(calc("log(1000)"), "3");
        assert_eq!(calc("deg(pi)"), "180");
        assert_eq!(calc("2pi"), "6.2831853072");
    }

    #[test]
    fn errors_are_reported_not_panicking() {
        assert!(calc("1/0").contains("delen door nul"));
        assert!(calc("2+").contains("onvolledig"));
        assert!(calc("(2+3").contains("sluithaakje"));
        assert!(calc("foo(2)").contains("onbekende naam"));
        assert!(calc("2 $ 3").contains("onverwacht teken"));
        assert!(calc("sqrt(-1)").contains("onbepaald"));
        assert!(calc("9^9^9").contains("onbepaald of te groot"));
        assert!(calc("").contains("geen som"));
        assert!(calc(&"1+".repeat(150)).contains("te lang"));
        assert!(calc(&format!("{}1{}", "(".repeat(90), ")".repeat(90))).contains("te veel geneste"));
    }

    #[test]
    fn number_formatting() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(1e20), "1e20");
        assert_eq!(format_number(-2.5), "-2.5");
    }
}
