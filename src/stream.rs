use std::char;
use std::str;

use {
    ErrorPos,
    StrSpan,
    XmlByteExt,
    XmlCharExt,
};


error_chain! {
    types {
        StreamError, StreamErrorKind, ResultExt, Result;
    }

    errors {
        /// The steam ended earlier than we expected.
        ///
        /// Should only appear on invalid input data.
        /// Errors in a valid XML should be handled by errors below.
        UnexpectedEndOfStream {
            display("unexpected end of stream")
        }

        /// An invalid Name.
        InvalidName {
            display("invalid name token")
        }

        /// An invalid/unexpected character in the stream.
        InvalidChar(current: char, expected: String, pos: ErrorPos) {
            display("expected '{}', found '{}' at {}", expected, current, pos)
        }

        /// An invalid reference.
        InvalidReference {
            display("invalid reference")
        }
    }
}


/// Representation of the [Reference](https://www.w3.org/TR/xml/#NT-Reference) value.
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Reference<'a> {
    /// An entity reference.
    ///
    /// https://www.w3.org/TR/xml/#NT-EntityRef
    EntityRef(StrSpan<'a>),
    /// A character reference.
    ///
    /// https://www.w3.org/TR/xml/#NT-CharRef
    CharRef(char),
}


/// Streaming text parsing interface.
#[derive(PartialEq, Clone, Copy, Debug)]
pub struct Stream<'a> {
    bytes: &'a [u8],
    pos: usize,
    end: usize,
    span: StrSpan<'a>,
}

impl<'a> Stream<'a> {
    /// Constructs a new `Stream` from a string span.
    pub fn from_span(span: StrSpan<'a>) -> Stream {
        Stream {
            bytes: span.to_str().as_bytes(),
            pos: 0,
            end: span.len(),
            span: span,
        }
    }

    /// Constructs a new `Stream` from a string.
    pub fn from_str(text: &str) -> Stream {
        Stream {
            bytes: text.as_bytes(),
            pos: 0,
            end: text.len(),
            span: StrSpan::from_str(text),
        }
    }

    /// Returns an underling string span.
    pub fn span(&self) -> StrSpan<'a> {
        self.span
    }

    /// Returns current position.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Sets current position equal to the end.
    ///
    /// Used to indicate end of parsing on error.
    pub fn jump_to_end(&mut self) {
        self.pos = self.end;
    }

    /// Checks if the stream is reached the end.
    ///
    /// Any [`pos()`] value larger than original text length indicates stream end.
    ///
    /// Accessing stream after reaching end via safe methods will produce
    /// an `UnexpectedEndOfStream` error.
    ///
    /// Accessing stream after reaching end via *_unchecked methods will produce
    /// a Rust's bound checking error.
    ///
    /// [`pos()`]: #method.pos
    #[inline]
    pub fn at_end(&self) -> bool {
        self.pos >= self.end
    }

    /// Returns a byte from a current stream position.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEndOfStream` if we are at the end of the stream.
    pub fn curr_byte(&self) -> Result<u8> {
        if self.at_end() {
            return Err(StreamErrorKind::UnexpectedEndOfStream.into());
        }

        Ok(self.curr_byte_unchecked())
    }

    /// Checks that current byte is equal to provided.
    ///
    /// Returns `false` if no bytes left.
    #[inline]
    pub fn is_curr_byte_eq(&self, c: u8) -> bool {
        if !self.at_end() {
            self.curr_byte_unchecked() == c
        } else {
            false
        }
    }

    #[inline]
    fn curr_byte_unchecked(&self) -> u8 {
        self.bytes[self.pos]
    }

    /// Returns a byte from a current stream position if there is one.
    #[inline]
    pub fn get_curr_byte(&self) -> Option<u8> {
        if !self.at_end() {
            Some(self.curr_byte_unchecked())
        } else {
            None
        }
    }

    /// Returns a next byte from a current stream position.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEndOfStream` if we are at the end of the stream.
    pub fn next_byte(&self) -> Result<u8> {
        if self.pos + 1 >= self.end {
            return Err(StreamErrorKind::UnexpectedEndOfStream.into());
        }

        Ok(self.bytes[self.pos + 1])
    }

    /// Returns a char from a current stream position.
    ///
    /// # Errors
    ///
    /// Returns `UnexpectedEndOfStream` if we are at the end of the stream.
    pub fn curr_char(&self) -> Result<char> {
        if self.at_end() {
            return Err(StreamErrorKind::UnexpectedEndOfStream.into());
        }

        Ok(self.curr_char_unchecked())
    }

    #[inline]
    fn curr_char_unchecked(&self) -> char {
        self.span.to_str()[self.pos..].chars().next().unwrap()
    }

    /// Advances by `n` bytes.
    ///
    /// # Examples
    ///
    /// ```rust,should_panic
    /// use xmlparser::Stream;
    ///
    /// let mut s = Stream::from_str("text");
    /// s.advance(2); // ok
    /// s.advance(20); // will cause a panic via debug_assert!().
    /// ```
    #[inline]
    pub fn advance(&mut self, n: usize) {
        debug_assert!(self.pos + n <= self.end);
        self.pos += n;
    }

    /// Skips whitespaces.
    ///
    /// Accepted values: `' ' \n \r \t &#x20; &#x9; &#xD; &#xA;`.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmlparser::Stream;
    ///
    /// let mut s = Stream::from_str(" \t\n\r &#x20; ");
    /// s.skip_spaces();
    /// assert_eq!(s.at_end(), true);
    /// ```
    pub fn skip_spaces(&mut self) {
        while !self.at_end() {
            let c = self.curr_byte_unchecked();

            if c.is_xml_space() {
                self.advance(1);
            } else if c == b'&' {
                // Check for (#x20 | #x9 | #xD | #xA).
                let start = self.pos();
                let mut is_space = false;
                if let Ok(Reference::CharRef(ch)) = self.consume_reference() {
                    if (ch as u32) < 255 && (ch as u8).is_xml_space() {
                        is_space = true;
                    }
                }

                if !is_space {
                    self.pos = start;
                    break;
                }
            } else {
                break;
            }
        }
    }

    /// Skips ASCII whitespaces.
    ///
    /// Accepted values: `' ' \n \r \t`.
    pub fn skip_ascii_spaces(&mut self) {
        while !self.at_end() {
            if self.curr_byte_unchecked().is_xml_space() {
                self.advance(1);
            } else {
                break;
            }
        }
    }

    /// Checks that the stream starts with a selected text.
    ///
    /// We are using `&[u8]` instead of `&str` for performance reasons.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmlparser::Stream;
    ///
    /// let mut s = Stream::from_str("Some text.");
    /// s.advance(5);
    /// assert_eq!(s.starts_with(b"text"), true);
    /// assert_eq!(s.starts_with(b"long"), false);
    /// ```
    #[inline]
    pub fn starts_with(&self, text: &[u8]) -> bool {
        self.bytes[self.pos..self.end].starts_with(text)
    }

    /// Checks if the stream is starts with a space.
    ///
    /// Uses [`skip_spaces()`](#method.curr_byte) internally.
    pub fn starts_with_space(&self) -> bool {
        if self.at_end() {
            return false;
        }

        let mut is_space = false;

        let c = self.curr_byte_unchecked();

        if c.is_xml_space() {
            is_space = true;
        } else if c == b'&' {
            // Check for (#x20 | #x9 | #xD | #xA).
            let mut s = self.clone();
            if let Some(v) = s.try_consume_char_reference() {
                if (v as u32) < 255 && (v as u8).is_xml_space() {
                    is_space = true;
                }
            }
        }

        is_space
    }

    /// Consumes whitespaces.
    ///
    /// Like [`skip_spaces()`], but checks that first char is actually a space.
    ///
    /// [`skip_spaces()`]: #method.skip_spaces
    pub fn consume_spaces(&mut self) -> Result<()> {
        if !self.at_end() && !self.starts_with_space() {
            let c = self.curr_byte_unchecked() as char;
            let pos = self.gen_error_pos();
            return Err(StreamErrorKind::InvalidChar(c, "Space".into(), pos).into());
        }

        self.skip_spaces();
        Ok(())
    }

    /// Consumes current byte if it's equal to the provided byte.
    ///
    /// # Errors
    ///
    /// - `InvalidChar`
    ///
    /// # Examples
    ///
    /// ```
    /// use xmlparser::Stream;
    ///
    /// let mut s = Stream::from_str("Some text.");
    /// s.consume_byte(b'S').unwrap();
    /// s.consume_byte(b'o').unwrap();
    /// s.consume_byte(b'm').unwrap();
    /// // s.consume_byte(b'q').unwrap(); // will produce an error
    /// ```
    pub fn consume_byte(&mut self, c: u8) -> Result<()> {
        if self.curr_byte()? != c {
            return Err(
                StreamErrorKind::InvalidChar(
                    self.curr_byte_unchecked() as char,
                    String::from_utf8(vec![c]).unwrap(),
                    self.gen_error_pos(),
                ).into()
            );
        }

        self.advance(1);
        Ok(())
    }

    /// Consumes current byte if it's equal to one of the provided bytes.
    ///
    /// Returns a coincidental byte.
    ///
    /// # Errors
    ///
    /// - `InvalidChar`
    pub fn consume_either(&mut self, list: &[u8]) -> Result<u8> {
        assert!(!list.is_empty());

        let c = self.curr_byte()?;
        if !list.contains(&c) {
            let s = String::from_utf8(list.to_vec()).unwrap();
            return Err(StreamErrorKind::InvalidChar(c as char, s, self.gen_error_pos()).into());
        }

        self.advance(1);
        Ok(c)
    }

    /// Consumes selected string.
    ///
    /// # Errors
    ///
    /// - `InvalidChar`
    pub fn skip_string(&mut self, text: &[u8]) -> Result<()> {
        // TODO: use custom error
        if !self.starts_with(text) {
            for c in text {
                self.consume_byte(*c)?;
            }

            // unreachable
        }

        self.advance(text.len());
        Ok(())
    }

    /// Consumes an XML name and returns it.
    ///
    /// Consumes according to: https://www.w3.org/TR/xml/#NT-Name
    ///
    /// # Errors
    ///
    /// - `InvalidNameToken` - if name is empty or starts with an invalid char
    /// - `UnexpectedEndOfStream`
    pub fn consume_name(&mut self) -> Result<StrSpan<'a>> {
        let start = self.pos();
        self.skip_name()?;

        let name = self.slice_back(start);
        if name.is_empty() {
            return Err(StreamErrorKind::InvalidName.into());
        }

        Ok(name)
    }

    /// Skips an XML name.
    ///
    /// The same as `consume_name()`, but does not return a consumed name.
    pub fn skip_name(&mut self) -> Result<()> {
        let mut iter = self.span.to_str()[self.pos..self.end].chars();
        if let Some(c) = iter.next() {
            if c.is_xml_name_start() {
                self.advance(c.len_utf8());
            } else {
                return Err(StreamErrorKind::InvalidName.into());
            }
        }

        for c in iter {
            if c.is_xml_name() {
                self.advance(c.len_utf8());
            } else {
                break;
            }
        }

        Ok(())
    }

    /// Consumes `=`.
    ///
    /// Consumes according to: https://www.w3.org/TR/xml/#NT-Eq
    ///
    /// # Errors
    ///
    /// - `InvalidChar`
    pub fn consume_eq(&mut self) -> Result<()> {
        self.skip_ascii_spaces();
        self.consume_byte(b'=')?;
        self.skip_ascii_spaces();

        Ok(())
    }

    /// Consumes quote.
    ///
    /// Consumes '\'' or '"' and returns it.
    ///
    /// # Errors
    ///
    /// - `InvalidChar`
    pub fn consume_quote(&mut self) -> Result<u8> {
        let c = self.curr_byte()?;
        if c == b'\'' || c == b'"' {
            self.advance(1);
            Ok(c)
        } else {
            Err(StreamErrorKind::InvalidChar(c as char, "'\"".into(), self.gen_error_pos()).into())
        }
    }

    /// Consumes bytes by predicate and returns them.
    ///
    /// Result can be empty.
    pub fn consume_bytes<F>(&mut self, f: F) -> StrSpan<'a>
        where F: Fn(&Stream, u8) -> bool
    {
        let start = self.pos();
        self.skip_bytes(f);
        self.slice_back(start)
    }

    /// Consumes bytes by predicate.
    pub fn skip_bytes<F>(&mut self, f: F)
        where F: Fn(&Stream, u8) -> bool
    {
        while !self.at_end() {
            let c = self.curr_byte_unchecked();
            if f(self, c) {
                self.advance(1);
            } else {
                break;
            }
        }
    }

    /// Consumes chars by predicate and returns them.
    ///
    /// Result can be empty.
    pub fn consume_chars<F>(&mut self, f: F) -> StrSpan<'a>
        where F: Fn(&Stream, char) -> bool
    {
        let start = self.pos();
        self.skip_chars(f);
        self.slice_back(start)
    }

    /// Consumes chars by predicate.
    pub fn skip_chars<F>(&mut self, f: F)
        where F: Fn(&Stream, char) -> bool
    {
        let t = &self.span.to_str()[self.pos..self.end];
        for c in t.chars() {
            if f(self, c) {
                self.advance(c.len_utf8());
            } else {
                break;
            }
        }
    }

    /// Consumes an XML character reference if there is one.
    ///
    /// On error will reset the position to the original.
    pub fn try_consume_char_reference(&mut self) -> Option<char> {
        let start = self.pos();

        match self.consume_reference() {
            Ok(Reference::CharRef(ch)) => Some(ch),
            _ => {
                self.pos = start;
                None
            }
        }
    }

    /// Consumes an XML reference.
    ///
    /// Consumes according to: https://www.w3.org/TR/xml/#NT-Reference
    pub fn consume_reference(&mut self) -> Result<Reference<'a>> {
        if self.curr_byte()? != b'&' {
            return Err(StreamErrorKind::InvalidReference.into());
        }

        self.advance(1);
        let reference = if self.curr_byte()? == b'#' {
            self.advance(1);
            let n = if self.curr_byte()? == b'x' {
                self.advance(1);
                let value = self.consume_bytes(|_, c| c.is_xml_hex_digit());
                match u32::from_str_radix(value.to_str(), 16) {
                    Ok(v) => v,
                    Err(_) => return Err(StreamErrorKind::InvalidReference.into()),
                }
            } else {
                let value = self.consume_bytes(|_, c| c.is_xml_digit());
                match u32::from_str_radix(value.to_str(), 10) {
                    Ok(v) => v,
                    Err(_) => return Err(StreamErrorKind::InvalidReference.into()),
                }
            };

            let c = char::from_u32(n).unwrap_or('\u{FFFD}');
            if c.is_xml_char() {
                Reference::CharRef(c)
            } else {
                return Err(StreamErrorKind::InvalidReference.into());
            }
        } else {
            let name = self.consume_name()?;
            match name.to_str() {
                "quot" => Reference::CharRef('"'),
                "amp"  => Reference::CharRef('&'),
                "apos" => Reference::CharRef('\''),
                "lt"   => Reference::CharRef('<'),
                "gt"   => Reference::CharRef('>'),
                _ => Reference::EntityRef(name),
            }
        };

        self.consume_byte(b';')?;

        Ok(reference)
    }

    /// Slices data from `pos` to the current position.
    pub fn slice_back(&mut self, pos: usize) -> StrSpan<'a> {
        self.span.slice_region(pos, self.pos())
    }

    /// Slices data from the current position to the end.
    pub fn slice_tail(&mut self) -> StrSpan<'a> {
        self.span.slice_region(self.pos(), self.end)
    }

    /// Calculates a current absolute position.
    ///
    /// This operation is very expensive. Use only for errors.
    pub fn gen_error_pos(&self) -> ErrorPos {
        let row = self.calc_current_row();
        let col = self.calc_current_col();
        ErrorPos::new(row, col)
    }

    /// Calculates a current absolute position.
    ///
    /// This operation is very expensive. Use only for errors.
    pub fn gen_error_pos_from(&mut self, pos: usize) -> ErrorPos {
        let old_pos = self.pos;
        self.pos = pos;
        let e = self.gen_error_pos();
        self.pos = old_pos;
        e
    }

    fn calc_current_row(&self) -> usize {
        let text = self.span.full_str();
        let mut row = 1;
        let end = self.pos + self.span.start();
        row += text.as_bytes()
            .iter()
            .take(end)
            .filter(|c| **c == b'\n')
            .count();
        row
    }

    fn calc_current_col(&self) -> usize {
        let text = self.span.full_str();
        let bytes = text.as_bytes();
        let end = self.pos + self.span.start();
        let mut col = 1;
        for n in 0..end {
            if n > 0 && bytes[n - 1] == b'\n' {
                col = 2;
            } else {
                col += 1;
            }
        }

        col
    }
}