//! A small, hand-written JSON parser and pretty-printer.
//!
//! GitWell stays dependency-free, but the triage subsystem needs to *read*
//! JSON it previously wrote to disk. This module is intentionally minimal:
//! recursive-descent parser into a `Value` enum, plus a pretty-printer.
//!
//! The existing ad-hoc JSON writer in `report.rs` is unchanged — this
//! module is used exclusively by [`crate::triage_state`].

use std::fmt::Write;

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    /// Object entries are stored as a Vec so we preserve insertion order
    /// on round-trips. Lookups are O(n) but n is tiny for our schema.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        if let Value::Object(entries) = self {
            entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Parser — recursive descent
// ---------------------------------------------------------------------------

pub fn parse(input: &str) -> Result<Value, String> {
    let mut p = Parser {
        input: input.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.input.len() {
        return Err(format!("unexpected trailing data at byte {}", p.pos));
    }
    Ok(v)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!(
                "expected '{}' at byte {}, got {:?}",
                b as char,
                self.pos,
                self.peek().map(|c| c as char)
            ))
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(format!("unexpected byte {:?} at {}", c as char, self.pos)),
            None => Err("unexpected EOF".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        self.skip_ws();
        let mut entries = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    return Err(format!(
                        "expected ',' or '}}' at byte {}",
                        self.pos
                    ));
                }
            }
        }
        Ok(Value::Object(entries))
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.pos)),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut s = String::new();
        while let Some(c) = self.peek() {
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(s);
                }
                b'\\' => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => {
                            s.push('"');
                            self.pos += 1;
                        }
                        Some(b'\\') => {
                            s.push('\\');
                            self.pos += 1;
                        }
                        Some(b'/') => {
                            s.push('/');
                            self.pos += 1;
                        }
                        Some(b'n') => {
                            s.push('\n');
                            self.pos += 1;
                        }
                        Some(b'r') => {
                            s.push('\r');
                            self.pos += 1;
                        }
                        Some(b't') => {
                            s.push('\t');
                            self.pos += 1;
                        }
                        Some(b'b') => {
                            s.push('\x08');
                            self.pos += 1;
                        }
                        Some(b'f') => {
                            s.push('\x0c');
                            self.pos += 1;
                        }
                        Some(b'u') => {
                            self.pos += 1;
                            if self.pos + 4 > self.input.len() {
                                return Err(format!("incomplete \\u escape at {}", self.pos));
                            }
                            let hex = std::str::from_utf8(&self.input[self.pos..self.pos + 4])
                                .map_err(|_| "invalid utf-8 in \\u escape".to_string())?;
                            let n = u32::from_str_radix(hex, 16)
                                .map_err(|_| format!("invalid \\u escape at {}", self.pos))?;
                            self.pos += 4;
                            if let Some(ch) = char::from_u32(n) {
                                s.push(ch);
                            }
                        }
                        _ => return Err(format!("bad escape at byte {}", self.pos)),
                    }
                }
                _ => {
                    // Copy one UTF-8 codepoint.
                    let start = self.pos;
                    self.pos += 1;
                    while self.pos < self.input.len() && (self.input[self.pos] & 0xC0) == 0x80 {
                        self.pos += 1;
                    }
                    let slice = std::str::from_utf8(&self.input[start..self.pos])
                        .map_err(|_| "invalid utf-8 in string".to_string())?;
                    s.push_str(slice);
                }
            }
        }
        Err("unterminated string".to_string())
    }

    fn parse_bool(&mut self) -> Result<Value, String> {
        if self.input[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(Value::Bool(true))
        } else if self.input[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(Value::Bool(false))
        } else {
            Err(format!("expected bool at byte {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<Value, String> {
        if self.input[self.pos..].starts_with(b"null") {
            self.pos += 4;
            Ok(Value::Null)
        } else {
            Err(format!("expected null at byte {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit()
                || c == b'.'
                || c == b'e'
                || c == b'E'
                || c == b'+'
                || c == b'-'
            {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| "invalid number".to_string())?;
        if s.contains('.') || s.contains('e') || s.contains('E') {
            s.parse::<f64>()
                .map(Value::Float)
                .map_err(|_| format!("invalid number {}", s))
        } else {
            s.parse::<i64>()
                .map(Value::Int)
                .map_err(|_| format!("invalid number {}", s))
        }
    }
}

// ---------------------------------------------------------------------------
// Pretty printer
// ---------------------------------------------------------------------------

pub fn to_pretty_string(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, 0);
    out.push('\n');
    out
}

fn write_value(out: &mut String, v: &Value, indent: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => {
            let _ = write!(out, "{}", n);
        }
        Value::Float(f) => {
            let _ = write!(out, "{}", f);
        }
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                out.push('\n');
                for _ in 0..(indent + 2) {
                    out.push(' ');
                }
                write_value(out, item, indent + 2);
                if i + 1 < items.len() {
                    out.push(',');
                }
            }
            out.push('\n');
            for _ in 0..indent {
                out.push(' ');
            }
            out.push(']');
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, val)) in entries.iter().enumerate() {
                out.push('\n');
                for _ in 0..(indent + 2) {
                    out.push(' ');
                }
                write_string(out, k);
                out.push_str(": ");
                write_value(out, val, indent + 2);
                if i + 1 < entries.len() {
                    out.push(',');
                }
            }
            out.push('\n');
            for _ in 0..indent {
                out.push(' ');
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_simple_object() {
        let input = r#"{"a": 1, "b": "hi", "c": [true, false, null]}"#;
        let v = parse(input).unwrap();
        assert_eq!(v.get("a").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(v.get("b").and_then(|v| v.as_str()), Some("hi"));
        let arr = v.get("c").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_bool(), Some(true));
    }

    #[test]
    fn round_trip_nested() {
        let input = r#"{"outer": {"inner": [1, 2, 3]}}"#;
        let v = parse(input).unwrap();
        let inner = v
            .get("outer")
            .and_then(|o| o.get("inner"))
            .and_then(|i| i.as_array())
            .unwrap();
        assert_eq!(inner.len(), 3);
        assert_eq!(inner[2].as_i64(), Some(3));
    }

    #[test]
    fn escapes_in_strings() {
        let v = parse(r#""line\nbreak""#).unwrap();
        assert_eq!(v.as_str(), Some("line\nbreak"));
    }

    #[test]
    fn pretty_round_trip() {
        let input = r#"{"version": 1, "decisions": [{"kind": "archive", "executed": false}]}"#;
        let v = parse(input).unwrap();
        let out = to_pretty_string(&v);
        // Re-parse the pretty output.
        let v2 = parse(&out).unwrap();
        assert_eq!(v2.get("version").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(
            v2.get("decisions")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .and_then(|o| o.get("kind"))
                .and_then(|v| v.as_str()),
            Some("archive")
        );
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse(r#"{"a": 1} junk"#).is_err());
    }
}
