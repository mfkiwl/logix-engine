//! Where things are in a JSON text, by path, so problems in a file the
//! request names (such as custom boards) come back as diagnostics at line
//! and column, as tools report problems in source files.

use std::collections::HashMap;

/// A line and column, both starting at 1. Columns count characters.
pub type Position = (u32, u32);

/// The positions of the values and object keys of a JSON text, by path:
/// "" is the root, "/my-board" a member of the root object, and
/// "/my-board/variants/0" the first element of its variants. A member's key
/// is under the same path in `keys`. Built only from valid JSON.
#[derive(Debug, Default)]
pub struct PositionIndex {
    values: HashMap<String, Position>,
    keys: HashMap<String, Position>,
}

impl PositionIndex {
    /// Indexes `text`, or None when it isn't valid JSON.
    pub fn new(text: &str) -> Option<PositionIndex> {
        let mut s = Scanner {
            chars: text.chars().collect(),
            at: 0,
            line: 1,
            column: 1,
            index: PositionIndex::default(),
        };
        s.value("")?;
        s.space();
        (s.at == s.chars.len()).then_some(s.index)
    }

    /// Where the value at `path` starts.
    pub fn value(&self, path: &str) -> Option<Position> {
        self.values.get(path).copied()
    }

    /// Where the key of the member at `path` starts.
    pub fn key(&self, path: &str) -> Option<Position> {
        self.keys.get(path).copied()
    }

    /// The value at `path`, else the closest enclosing value that has a
    /// position, such as the object a missing field belongs in.
    pub fn nearest(&self, path: &str) -> Option<Position> {
        let mut p = path;
        loop {
            if let Some(pos) = self.value(p) {
                return Some(pos);
            }
            p = &p[..p.rfind('/')?];
        }
    }
}

/// A path's member, escaped so a key with "/" can't be mistaken for two.
pub fn segment(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

struct Scanner {
    chars: Vec<char>,
    at: usize,
    line: u32,
    column: u32,
    index: PositionIndex,
}

impl Scanner {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn space(&mut self) {
        while self
            .peek()
            .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
        {
            self.bump();
        }
    }

    fn expect(&mut self, c: char) -> Option<()> {
        (self.bump()? == c).then_some(())
    }

    fn value(&mut self, path: &str) -> Option<()> {
        self.space();
        self.index
            .values
            .insert(path.to_string(), (self.line, self.column));
        match self.peek()? {
            '{' => self.object(path),
            '[' => self.array(path),
            '"' => self.string().map(|_| ()),
            _ => self.scalar(),
        }
    }

    fn object(&mut self, path: &str) -> Option<()> {
        self.expect('{')?;
        self.space();
        if self.peek()? == '}' {
            self.bump();
            return Some(());
        }
        loop {
            self.space();
            let at = (self.line, self.column);
            let key = self.string()?;
            let member = format!("{path}/{}", segment(&key));
            self.index.keys.insert(member.clone(), at);
            self.space();
            self.expect(':')?;
            self.value(&member)?;
            self.space();
            match self.bump()? {
                ',' => continue,
                '}' => return Some(()),
                _ => return None,
            }
        }
    }

    fn array(&mut self, path: &str) -> Option<()> {
        self.expect('[')?;
        self.space();
        if self.peek()? == ']' {
            self.bump();
            return Some(());
        }
        let mut i = 0;
        loop {
            self.value(&format!("{path}/{i}"))?;
            i += 1;
            self.space();
            match self.bump()? {
                ',' => continue,
                ']' => return Some(()),
                _ => return None,
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.bump()? {
                '"' => return Some(out),
                '\\' => {
                    let c = self.bump()?;
                    match c {
                        'u' => {
                            let hex: String = (0..4).map(|_| self.bump()).collect::<Option<_>>()?;
                            out.push(
                                char::from_u32(u32::from_str_radix(&hex, 16).ok()?).unwrap_or('?'),
                            );
                        }
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
    }

    /// A number, true, false or null.
    fn scalar(&mut self) -> Option<()> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.'))
        {
            self.bump();
        }
        (self.at > start).then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_values_and_keys_by_path() {
        let text = "{\n  \"my-board\": {\n    \"variants\": [\n      {\"usb\": {\"vid\": \"x\"}}\n    ]\n  },\n  \"a/b\": 1\n}\n";
        let ix = PositionIndex::new(text).unwrap();
        assert_eq!(ix.value(""), Some((1, 1)));
        assert_eq!(ix.key("/my-board"), Some((2, 3)));
        assert_eq!(ix.value("/my-board"), Some((2, 15)));
        assert_eq!(ix.value("/my-board/variants/0"), Some((4, 7)));
        assert_eq!(ix.value("/my-board/variants/0/usb/vid"), Some((4, 23)));
        assert_eq!(ix.value(&format!("/{}", segment("a/b"))), Some((7, 10)));
        // A missing field: the object it belongs in.
        assert_eq!(ix.nearest("/my-board/fpga_id"), Some((2, 15)));
        assert_eq!(ix.nearest("/my-board/variants/0/usb/pid"), Some((4, 15)));
    }

    #[test]
    fn counts_characters_and_escapes() {
        let ix = PositionIndex::new("{\"é\\\"\": [true, null, -1.5e3, \"\\u00e9\"]}").unwrap();
        assert_eq!(ix.value("/é\"/2"), Some((1, 22)));
        assert_eq!(ix.value("/é\"/3"), Some((1, 30)));
        assert!(PositionIndex::new("{").is_none());
        assert!(PositionIndex::new("{} x").is_none());
    }
}
