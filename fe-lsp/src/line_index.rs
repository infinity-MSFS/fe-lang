//! Byte offsets to protocol positions and back.
//!
//! `fe-lang` spans are byte offsets. The protocol speaks line and character,
//! where "character" means UTF-16 code units by default — not bytes, and not
//! `char`s either. For `notify "cabin altitude 10 000 ft ⚠"` those three counts
//! are three different numbers, and getting it wrong puts the squiggle in the
//! wrong place with no error anywhere to explain why.
//!
//! Which encoding applies is negotiated at initialize, so the index carries it.

use lsp_types::{Position, PositionEncodingKind, Range};

use fe_lang::span::Span;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Encoding {
    /// The protocol's default, and what every client supports.
    #[default]
    Utf16,
    /// Offered by clients that would rather not convert; then a byte offset is
    /// the answer already.
    Utf8,
}

impl Encoding {
    pub fn negotiate(offered: Option<&[PositionEncodingKind]>) -> Encoding {
        match offered {
            Some(kinds) if kinds.contains(&PositionEncodingKind::UTF8) => Encoding::Utf8,
            _ => Encoding::Utf16,
        }
    }

    pub fn kind(self) -> PositionEncodingKind {
        match self {
            Encoding::Utf8 => PositionEncodingKind::UTF8,
            Encoding::Utf16 => PositionEncodingKind::UTF16,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LineIndex {
    /// Byte offset of the first character of each line. Always starts with 0.
    line_starts: Vec<u32>,
    len: u32,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset as u32 + 1);
            }
        }
        LineIndex {
            line_starts,
            len: text.len() as u32,
        }
    }

    /// Byte offset of `position`, clamped into the document.
    ///
    /// Clamping rather than failing is deliberate: a position can arrive one
    /// keystroke stale, and answering about the nearest real place in the file
    /// is better than dropping the request.
    pub fn offset(&self, text: &str, position: Position, encoding: Encoding) -> u32 {
        let line = position.line as usize;
        let Some(&line_start) = self.line_starts.get(line) else {
            return self.len;
        };
        let line_end = self
            .line_starts
            .get(line + 1)
            .map(|&next| next.saturating_sub(1))
            .unwrap_or(self.len);
        let line_text = &text[line_start as usize..line_end.min(self.len) as usize];

        let mut remaining = position.character as usize;
        let mut offset = line_start as usize;
        for ch in line_text.chars() {
            let width = match encoding {
                Encoding::Utf8 => ch.len_utf8(),
                Encoding::Utf16 => ch.len_utf16(),
            };
            if remaining < width {
                break;
            }
            remaining -= width;
            offset += ch.len_utf8();
        }
        offset as u32
    }

    pub fn position(&self, text: &str, offset: u32, encoding: Encoding) -> Position {
        let offset = offset.min(self.len);
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line] as usize;

        // `offset` comes from the lexer and so is always a character boundary,
        // but a clamped one need not be; walk characters rather than slicing.
        let mut character = 0usize;
        for (index, ch) in text[line_start..].char_indices() {
            if line_start + index >= offset as usize {
                break;
            }
            character += match encoding {
                Encoding::Utf8 => ch.len_utf8(),
                Encoding::Utf16 => ch.len_utf16(),
            };
        }

        Position {
            line: line as u32,
            character: character as u32,
        }
    }

    pub fn range(&self, text: &str, span: Span, encoding: Encoding) -> Range {
        Range {
            start: self.position(text, span.start, encoding),
            end: self.position(text, span.end, encoding),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fe_lang::span::UnitId;

    fn at(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[track_caller]
    fn round_trip(text: &str, offset: u32, encoding: Encoding, expected: Position) {
        let index = LineIndex::new(text);
        assert_eq!(index.position(text, offset, encoding), expected);
        assert_eq!(index.offset(text, expected, encoding), offset);
    }

    #[test]
    fn ascii_lines() {
        let text = "procedure P {\n    complete\n}\n";
        round_trip(text, 0, Encoding::Utf16, at(0, 0));
        round_trip(text, 10, Encoding::Utf16, at(0, 10));
        round_trip(text, 14, Encoding::Utf16, at(1, 0));
        round_trip(text, 18, Encoding::Utf16, at(1, 4));
        round_trip(text, 27, Encoding::Utf16, at(2, 0));
        round_trip(text, 28, Encoding::Utf16, at(2, 1));
    }

    /// The case the encoding exists for. `é` is 2 bytes, 1 UTF-16 unit; the
    /// warning sign is 3 bytes, 1 unit; an emoji is 4 bytes and 2 units. Every
    /// column after one of them differs between the three ways of counting.
    #[test]
    fn a_multibyte_character_moves_the_column() {
        let text = "notify \"é⚠\" x";
        let byte_offset = text.find('x').unwrap() as u32;
        assert_eq!(byte_offset, 15);

        round_trip(text, byte_offset, Encoding::Utf16, at(0, 12));
        round_trip(text, byte_offset, Encoding::Utf8, at(0, 15));
    }

    #[test]
    fn an_astral_character_is_two_utf16_units() {
        let text = "notify \"🛩\" x";
        let byte_offset = text.find('x').unwrap() as u32;

        // 8 before the emoji, +2 units for it, +2 for `" `
        round_trip(text, byte_offset, Encoding::Utf16, at(0, 12));
        round_trip(text, byte_offset, Encoding::Utf8, at(0, 14));
    }

    #[test]
    fn positions_past_the_end_clamp_instead_of_panicking() {
        let text = "procedure P {}\n";
        let index = LineIndex::new(text);
        assert_eq!(
            index.offset(text, at(99, 0), Encoding::Utf16),
            text.len() as u32
        );
        assert_eq!(index.offset(text, at(0, 999), Encoding::Utf16), 14);
        assert_eq!(index.position(text, 9999, Encoding::Utf16), at(1, 0));
    }

    /// A character offset landing inside a multi-byte character must not slice
    /// through it — a stale position from the client can do exactly that.
    #[test]
    fn a_position_inside_a_character_does_not_panic() {
        let text = "notify \"🛩\"";
        let index = LineIndex::new(text);
        for character in 0..20 {
            let offset = index.offset(text, at(0, character), Encoding::Utf16);
            assert!(
                text.is_char_boundary(offset as usize),
                "{character} -> {offset}"
            );
        }
        for offset in 0..text.len() as u32 + 4 {
            let _ = index.position(text, offset, Encoding::Utf16);
        }
    }

    #[test]
    fn a_span_becomes_a_range() {
        let text = "procedure P {\n    complete\n}\n";
        let index = LineIndex::new(text);
        let span = Span::new(UnitId(0), 18, 26);
        assert_eq!(&text[18..26], "complete");
        let range = index.range(text, span, Encoding::Utf16);
        assert_eq!(range.start, at(1, 4));
        assert_eq!(range.end, at(1, 12));
    }

    #[test]
    fn utf8_is_taken_only_when_the_client_offers_it() {
        assert_eq!(Encoding::negotiate(None), Encoding::Utf16);
        assert_eq!(Encoding::negotiate(Some(&[])), Encoding::Utf16);
        assert_eq!(
            Encoding::negotiate(Some(&[PositionEncodingKind::UTF16])),
            Encoding::Utf16
        );
        assert_eq!(
            Encoding::negotiate(Some(&[
                PositionEncodingKind::UTF8,
                PositionEncodingKind::UTF16
            ])),
            Encoding::Utf8
        );
    }
}
