// SNBT parser — Mojang's stringified-NBT format, with FTB Library's
// extensions: newlines accepted as compound/list separators, `//`/`#` line
// comments, and `=` accepted as the key-value separator.
//
// Implementation mirrors `dev.ftb.mods.ftblibrary.snbt.SNBTParser` from
// FTB Library so the dialect quirks match what real FTB worlds produce. The
// public API is a single `parse(&str) -> Result<SnbtValue, ParseError>`;
// callers walk the resulting tree using the helper accessors on `SnbtValue`.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum SnbtValue {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    List(Vec<SnbtValue>),
    Compound(HashMap<String, SnbtValue>),
}

impl SnbtValue {
    pub fn as_compound(&self) -> Option<&HashMap<String, SnbtValue>> {
        match self {
            Self::Compound(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[SnbtValue]> {
        match self {
            Self::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Byte(v) => Some(*v as i64),
            Self::Short(v) => Some(*v as i64),
            Self::Int(v) => Some(*v as i64),
            Self::Long(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_i32(&self) -> Option<i32> {
        self.as_i64().and_then(|v| i32::try_from(v).ok())
    }
    #[allow(dead_code)] // public accessor — used by tests, kept for callers
    pub fn as_int_array(&self) -> Option<&[i32]> {
        match self {
            Self::IntArray(a) => Some(a),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub row: usize,
    pub col: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "snbt parse error: {} @ {}:{}", self.message, self.row, self.col)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(input: &str) -> Result<SnbtValue, ParseError> {
    let buffer: Vec<char> = strip_comments(input).chars().collect();
    let mut p = Parser {
        buffer: &buffer,
        pos: 0,
    };
    let first = p.next_ns()?;
    p.read_tag(first)
}

/// Strip `//` and `#` line comments. Comment markers must come at the start
/// of the line (after leading whitespace) — matching FTB's behavior.
fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("//") && !trimmed.starts_with('#') {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

struct Parser<'a> {
    buffer: &'a [char],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn pos_info(&self, p: usize) -> (usize, usize) {
        let mut row = 0usize;
        let mut col = 0usize;
        for &c in &self.buffer[..p.min(self.buffer.len())] {
            if c == '\n' {
                row += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (row + 1, col + 1)
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        let (row, col) = self.pos_info(self.pos);
        ParseError {
            message: msg.into(),
            row,
            col,
        }
    }

    fn err_at(&self, p: usize, msg: impl Into<String>) -> ParseError {
        let (row, col) = self.pos_info(p);
        ParseError {
            message: msg.into(),
            row,
            col,
        }
    }

    fn next(&mut self) -> Result<char, ParseError> {
        if self.pos >= self.buffer.len() {
            return Err(self.err("unexpected EOF"));
        }
        let c = self.buffer[self.pos];
        self.pos += 1;
        Ok(c)
    }

    /// Mirrors Java's `c > ' '` test — skips ASCII whitespace and any other
    /// char with code <= 0x20 (space, tab, CR, LF, etc.).
    fn next_ns(&mut self) -> Result<char, ParseError> {
        loop {
            let c = self.next()?;
            if c as u32 > b' ' as u32 {
                return Ok(c);
            }
        }
    }

    fn read_tag(&mut self, first: char) -> Result<SnbtValue, ParseError> {
        match first {
            '{' => self.read_compound(),
            '[' => self.read_collection(),
            '"' => self.read_quoted_string('"').map(SnbtValue::String),
            '\'' => self.read_quoted_string('\'').map(SnbtValue::String),
            _ => {
                let s = self.read_word_string(first);
                Ok(parse_word(&s))
            }
        }
    }

    fn read_compound(&mut self) -> Result<SnbtValue, ParseError> {
        let mut map = HashMap::new();
        loop {
            let c = self.next_ns()?;
            if c == '}' {
                return Ok(SnbtValue::Compound(map));
            }
            // FTB writes one entry per line with no commas; Mojang uses
            // commas. Both end up as whitespace skipped by `next_ns`, so any
            // stray `,` we land on here is a Mojang-style separator.
            if c == ',' {
                continue;
            }
            let key = if c == '"' {
                self.read_quoted_string('"')?
            } else if c == '\'' {
                self.read_quoted_string('\'')?
            } else {
                self.read_word_string(c)
            };
            let n = self.next_ns()?;
            if n != ':' && n != '=' {
                return Err(self.err(format!("expected ':' or '=' after key, got '{}'", n)));
            }
            let next_first = self.next_ns()?;
            let value = self.read_tag(next_first)?;
            map.insert(key, value);
        }
    }

    fn read_collection(&mut self) -> Result<SnbtValue, ParseError> {
        let prev_pos = self.pos;
        let n1 = self.next_ns()?;
        // Empty list (`[]` or `[ ]`) — bail out before peeking n2, which would
        // EOF if the list is the last thing in the input.
        if n1 == ']' {
            return Ok(SnbtValue::List(Vec::new()));
        }
        // Two-char peek to distinguish `[I; ...]` from `[ <item>, ... ]`.
        let n2 = match self.next_ns() {
            Ok(c) => c,
            Err(_) => {
                self.pos = prev_pos;
                return self.read_list();
            }
        };
        if n2 == ';' && matches!(n1, 'I' | 'i' | 'L' | 'l' | 'B' | 'b') {
            self.read_typed_array(prev_pos, n1.to_ascii_lowercase())
        } else {
            self.pos = prev_pos;
            self.read_list()
        }
    }

    fn read_list(&mut self) -> Result<SnbtValue, ParseError> {
        let mut list = Vec::new();
        loop {
            let c = self.next_ns()?;
            if c == ']' {
                return Ok(SnbtValue::List(list));
            }
            if c == ',' {
                continue;
            }
            list.push(self.read_tag(c)?);
        }
    }

    fn read_typed_array(&mut self, header_pos: usize, typ: char) -> Result<SnbtValue, ParseError> {
        let mut ints = Vec::new();
        let mut longs = Vec::new();
        let mut bytes = Vec::new();
        loop {
            let c = self.next_ns()?;
            if c == ']' {
                return Ok(match typ {
                    'i' => SnbtValue::IntArray(ints),
                    'l' => SnbtValue::LongArray(longs),
                    'b' => SnbtValue::ByteArray(bytes),
                    _ => unreachable!("typ guarded by read_collection"),
                });
            }
            if c == ',' {
                continue;
            }
            let val = self.read_tag(c)?;
            let n = match val.as_i64() {
                Some(n) => n,
                None => {
                    return Err(self.err_at(header_pos, "non-numeric value in typed array"));
                }
            };
            match typ {
                'i' => ints.push(n as i32),
                'l' => longs.push(n),
                'b' => bytes.push(n as i8),
                _ => unreachable!(),
            }
        }
    }

    fn read_word_string(&mut self, first: char) -> String {
        let mut s = String::new();
        s.push(first);
        while self.pos < self.buffer.len() {
            let c = self.buffer[self.pos];
            if is_simple_char(c) {
                s.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        s
    }

    fn read_quoted_string(&mut self, stop: char) -> Result<String, ParseError> {
        let mut s = String::new();
        let mut escape = false;
        loop {
            let c = self.next()?;
            if c == '\n' && !escape {
                return Err(self.err_at(self.pos - 1, format!("newline inside {}-quoted string", stop)));
            }
            if escape {
                escape = false;
                let unescaped = match c {
                    '"' => '"',
                    '\\' => '\\',
                    '\'' => '\'',
                    't' => '\t',
                    'b' => '\u{08}',
                    'n' => '\n',
                    'r' => '\r',
                    'f' => '\u{0C}',
                    other => other,
                };
                s.push(unescaped);
            } else if c == '\\' {
                escape = true;
            } else if c == stop {
                return Ok(s);
            } else {
                s.push(c);
            }
        }
    }
}

fn is_simple_char(c: char) -> bool {
    c.is_ascii_alphabetic()
        || c.is_ascii_digit()
        || c == '.'
        || c == '_'
        || c == '-'
        || c == '+'
        || c == '∞'
}

/// Map a bare word to a tag value, mirroring FTB's `readTag` switch on the
/// fully-collected word. Order matters: `true`/`false` → byte, infinity/NaN
/// keywords → float/double, otherwise number-or-string detection.
fn parse_word(s: &str) -> SnbtValue {
    match s {
        "true" => return SnbtValue::Byte(1),
        "false" => return SnbtValue::Byte(0),
        "Infinity" | "Infinityd" | "+Infinity" | "+Infinityd" | "∞" | "∞d" | "+∞" | "+∞d" => {
            return SnbtValue::Double(f64::INFINITY);
        }
        "-Infinity" | "-Infinityd" | "-∞" | "-∞d" => return SnbtValue::Double(f64::NEG_INFINITY),
        "NaN" | "NaNd" => return SnbtValue::Double(f64::NAN),
        "Infinityf" | "+Infinityf" | "∞f" | "+∞f" => return SnbtValue::Float(f32::INFINITY),
        "-Infinityf" | "-∞f" => return SnbtValue::Float(f32::NEG_INFINITY),
        "NaNf" => return SnbtValue::Float(f32::NAN),
        _ => {}
    }
    parse_number(s)
}

fn parse_number(s: &str) -> SnbtValue {
    if s.is_empty() {
        return SnbtValue::String(String::new());
    }
    let last = s.chars().last().unwrap();
    let last_lower = last.to_ascii_lowercase();

    // Suffix-less integer: last char is a digit and the whole string parses
    // as i32. Matches FTB/Mojang precedence.
    if last.is_ascii_digit() {
        if let Ok(n) = s.parse::<i32>() {
            return SnbtValue::Int(n);
        }
    }

    if !s.is_empty() && matches!(last_lower, 'b' | 's' | 'l' | 'f' | 'd') {
        let body = &s[..s.len() - 1];
        match last_lower {
            'b' => {
                if let Ok(n) = body.parse::<i32>() {
                    return SnbtValue::Byte(n as i8);
                }
            }
            's' => {
                if let Ok(n) = body.parse::<i32>() {
                    return SnbtValue::Short(n as i16);
                }
            }
            'l' => {
                if let Ok(n) = body.parse::<i64>() {
                    return SnbtValue::Long(n);
                }
            }
            'f' => {
                if let Ok(n) = body.parse::<f32>() {
                    return SnbtValue::Float(n);
                }
            }
            'd' => {
                if let Ok(n) = body.parse::<f64>() {
                    return SnbtValue::Double(n);
                }
            }
            _ => {}
        }
    }

    // No suffix, but parses as a float-with-decimal-or-exponent. Matches
    // Mojang's "default to double" rule.
    if (s.contains('.') || s.contains('e') || s.contains('E')) && s.parse::<f64>().is_ok() {
        return SnbtValue::Double(s.parse::<f64>().unwrap());
    }

    SnbtValue::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(s: &str) -> SnbtValue {
        parse(s).unwrap_or_else(|e| panic!("parse failed: {} (input: {:?})", e, s))
    }

    #[test]
    fn empty_compound() {
        assert!(matches!(parse_ok("{}"), SnbtValue::Compound(m) if m.is_empty()));
        assert!(matches!(parse_ok("{ }"), SnbtValue::Compound(m) if m.is_empty()));
        assert!(matches!(parse_ok("{\n}"), SnbtValue::Compound(m) if m.is_empty()));
    }

    #[test]
    fn empty_list() {
        assert!(matches!(parse_ok("[]"), SnbtValue::List(l) if l.is_empty()));
        assert!(matches!(parse_ok("[ ]"), SnbtValue::List(l) if l.is_empty()));
    }

    #[test]
    fn simple_kv_mojang_commas() {
        let v = parse_ok("{x:5,z:-3,t:1700000000L}");
        let m = v.as_compound().unwrap();
        assert_eq!(m["x"].as_i32(), Some(5));
        assert_eq!(m["z"].as_i32(), Some(-3));
        assert_eq!(m["t"].as_i64(), Some(1_700_000_000));
    }

    #[test]
    fn simple_kv_ftb_newlines() {
        let v = parse_ok("{\n\tx: 5\n\tz: -3\n\tt: 1700000000L\n}");
        let m = v.as_compound().unwrap();
        assert_eq!(m["x"].as_i32(), Some(5));
        assert_eq!(m["z"].as_i32(), Some(-3));
        assert_eq!(m["t"].as_i64(), Some(1_700_000_000));
    }

    #[test]
    fn bare_uuid_key() {
        let v = parse_ok("{1ccb5e0e-75d7-4752-ac17-c4cc215971d8: \"owner\"}");
        let m = v.as_compound().unwrap();
        assert_eq!(
            m["1ccb5e0e-75d7-4752-ac17-c4cc215971d8"].as_str(),
            Some("owner")
        );
    }

    #[test]
    fn quoted_key_with_colon() {
        let v = parse_ok(r#"{"ftbteams:display_name": "Alice"}"#);
        assert_eq!(
            v.as_compound().unwrap()["ftbteams:display_name"].as_str(),
            Some("Alice")
        );
    }

    #[test]
    fn nested_compound_and_list() {
        let v = parse_ok("{chunks: {\n\"minecraft:overworld\": [\n{x: 1, z: 2}\n{x: 3, z: 4}\n]\n}}");
        let m = v.as_compound().unwrap();
        let chunks = m["chunks"].as_compound().unwrap();
        let ow = chunks["minecraft:overworld"].as_list().unwrap();
        assert_eq!(ow.len(), 2);
        assert_eq!(ow[0].as_compound().unwrap()["x"].as_i32(), Some(1));
        assert_eq!(ow[1].as_compound().unwrap()["z"].as_i32(), Some(4));
    }

    #[test]
    fn typed_int_array() {
        let v = parse_ok("[I; 1, -2, 3, 4]");
        assert_eq!(v.as_int_array(), Some(&[1, -2, 3, 4][..]));
    }

    #[test]
    fn typed_int_array_newlines() {
        let v = parse_ok("[I;\n  1\n  2\n  3\n  4\n]");
        assert_eq!(v.as_int_array(), Some(&[1, 2, 3, 4][..]));
    }

    #[test]
    fn list_of_strings() {
        let v = parse_ok(r#"["a", "b", "c"]"#);
        let l = v.as_list().unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].as_str(), Some("a"));
    }

    #[test]
    fn boolean_to_byte() {
        let v = parse_ok("{a: true, b: false}");
        let m = v.as_compound().unwrap();
        assert_eq!(m["a"], SnbtValue::Byte(1));
        assert_eq!(m["b"], SnbtValue::Byte(0));
    }

    #[test]
    fn number_suffixes() {
        let v = parse_ok("{a: 5b, b: 5s, c: 5L, d: 5.5f, e: 5.5d, f: 5}");
        let m = v.as_compound().unwrap();
        assert_eq!(m["a"], SnbtValue::Byte(5));
        assert_eq!(m["b"], SnbtValue::Short(5));
        assert_eq!(m["c"], SnbtValue::Long(5));
        assert!(matches!(m["d"], SnbtValue::Float(f) if (f - 5.5).abs() < 1e-6));
        assert!(matches!(m["e"], SnbtValue::Double(f) if (f - 5.5).abs() < 1e-9));
        assert_eq!(m["f"], SnbtValue::Int(5));
    }

    #[test]
    fn implicit_double_with_decimal() {
        assert_eq!(parse_ok("1.5"), SnbtValue::Double(1.5));
    }

    #[test]
    fn quoted_string_with_escapes() {
        let v = parse_ok(r#""a\"b\n""#);
        assert_eq!(v.as_str(), Some("a\"b\n"));
    }

    #[test]
    fn line_comment_skipped() {
        let v = parse_ok("// header comment\n{x: 5}\n# trailing comment");
        assert_eq!(v.as_compound().unwrap()["x"].as_i32(), Some(5));
    }

    #[test]
    fn equals_sign_works_as_kv_separator() {
        let v = parse_ok("{x = 5}");
        assert_eq!(v.as_compound().unwrap()["x"].as_i32(), Some(5));
    }

    #[test]
    fn ftb_real_team_file_shape() {
        let s = "{\n\tid: \"1ccb5e0e-75d7-4752-ac17-c4cc215971d8\"\n\ttype: \"player\"\n\tplayer_name: \"ZB0at\"\n\tranks: {\n\t\t1ccb5e0e-75d7-4752-ac17-c4cc215971d8: \"owner\"\n\t}\n\tproperties: {\n\t\t\"ftbteams:display_name\": \"ZB0at\"\n\t\t\"ftbteams:free_to_join\": 0b\n\t}\n\tmessage_history: [ ]\n\textra: { }\n}";
        let v = parse_ok(s);
        let m = v.as_compound().unwrap();
        assert_eq!(m["id"].as_str(), Some("1ccb5e0e-75d7-4752-ac17-c4cc215971d8"));
        assert_eq!(m["type"].as_str(), Some("player"));
        assert_eq!(m["player_name"].as_str(), Some("ZB0at"));
        assert_eq!(
            m["ranks"].as_compound().unwrap()["1ccb5e0e-75d7-4752-ac17-c4cc215971d8"]
                .as_str(),
            Some("owner")
        );
        assert_eq!(m["message_history"].as_list().unwrap().len(), 0);
    }

    #[test]
    fn id_as_int_array_uuid() {
        // 1.21.x onward writes top-level UUIDs as int[4]. Make sure we
        // accept the syntax even though we won't decode it back to a UUID
        // here (consumers that want UUIDs will look at map keys instead).
        let v = parse_ok("{id: [I; 1, 2, 3, 4]}");
        assert_eq!(
            v.as_compound().unwrap()["id"].as_int_array(),
            Some(&[1, 2, 3, 4][..])
        );
    }

    /// Walks the SNBT research corpus and parses every `.snbt` file. Run with
    /// `cargo test --release -- --ignored snbt_corpus` (or override the
    /// corpus root via the `MCMAP_SNBT_CORPUS` env var). Auto-skips if the
    /// corpus is unavailable, so it's safe to run in CI without fixtures.
    #[test]
    #[ignore]
    fn snbt_corpus_round_trip() {
        let root = std::env::var("MCMAP_SNBT_CORPUS")
            .unwrap_or_else(|_| "D:/temp/ftb-claim-research".to_string());
        let root_path = std::path::Path::new(&root);
        if !root_path.is_dir() {
            eprintln!("skip: corpus root {} not found", root);
            return;
        }

        let mut files: Vec<std::path::PathBuf> = Vec::new();
        walk_snbt(root_path, &mut files);
        assert!(!files.is_empty(), "no .snbt files found under {}", root);

        let mut failures: Vec<(std::path::PathBuf, String)> = Vec::new();
        for path in &files {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    failures.push((path.clone(), format!("read error: {}", e)));
                    continue;
                }
            };
            let s = match std::str::from_utf8(&bytes) {
                Ok(s) => s,
                Err(e) => {
                    failures.push((path.clone(), format!("utf-8 error: {}", e)));
                    continue;
                }
            };
            if let Err(e) = parse(s) {
                failures.push((path.clone(), e.to_string()));
            }
        }

        eprintln!(
            "snbt corpus: {} files, {} parsed, {} failed",
            files.len(),
            files.len() - failures.len(),
            failures.len()
        );
        for (p, e) in &failures {
            eprintln!("  FAIL {} :: {}", p.display(), e);
        }
        assert!(failures.is_empty(), "{} SNBT files failed to parse", failures.len());
    }

    fn walk_snbt(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_snbt(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("snbt") {
                out.push(path);
            }
        }
    }
}
