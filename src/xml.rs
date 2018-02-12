use std::fmt;

use error::ResultExt;

use error::Result;
use {
    ElementEnd,
    EntityDefinition,
    ErrorKind,
    ExternalId,
    FromSpan,
    Stream,
    StreamError,
    StreamErrorKind,
    StrSpan,
    Token,
    XmlByteExt,
    XmlCharExt,
};


#[derive(Clone, Copy, PartialEq)]
enum State {
    Document,
    Dtd,
    Elements,
    Attributes,
    AfterElements,
    Finished,
}


#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TokenType {
    XMLDecl,
    Comment,
    PI,
    DoctypeDecl,
    ElementDecl,
    AttlistDecl,
    EntityDecl,
    NotationDecl,
    DoctypeEnd,
    ElementStart,
    ElementClose,
    Attribute,
    CDSect,
    Whitespace,
    CharData,
}

impl fmt::Display for TokenType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match *self {
            TokenType::XMLDecl => "Declaration",
            TokenType::Comment => "Comment",
            TokenType::PI => "Processing Instruction",
            TokenType::DoctypeDecl => "Doctype Declaration",
            TokenType::ElementDecl => "Doctype Element Declaration",
            TokenType::AttlistDecl => "Doctype Attributes Declaration",
            TokenType::EntityDecl => "Doctype Entity Declaration",
            TokenType::NotationDecl => "Doctype Notation Declaration",
            TokenType::DoctypeEnd => "Doctype End",
            TokenType::ElementStart => "Element Start",
            TokenType::ElementClose => "Element Close",
            TokenType::Attribute => "Attribute",
            TokenType::CDSect => "CDATA",
            TokenType::Whitespace => "Whitespace",
            TokenType::CharData => "Character data",
        };

        write!(f, "{}", s)
    }
}


/// Tokenizer of the XML structure.
pub struct Tokenizer<'a> {
    stream: Stream<'a>,
    state: State,
    depth: usize,
}

impl<'a> FromSpan<'a> for Tokenizer<'a> {
    fn from_span(span: StrSpan<'a>) -> Self {
        Tokenizer {
            stream: Stream::from_span(span),
            state: State::Document,
            depth: 0,
        }
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Result<Token<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stream.at_end() || self.state == State::Finished {
            self.state = State::Finished;
            return None;
        }

        let t = Self::parse_next_impl(&mut self.stream, self.state);

        if let Some(ref t) = t {
            match *t {
                Ok(Token::ElementStart(_)) => {
                    self.state = State::Attributes;
                }
                Ok(Token::ElementEnd(ref end)) => {
                    match *end {
                        ElementEnd::Open => {
                            self.depth += 1;
                        }
                        ElementEnd::Close(_) => {
                            if self.depth > 0 {
                                self.depth -= 1;
                            }
                        }
                        ElementEnd::Empty => {}
                    }

                    if self.depth == 0 {
                        self.state = State::AfterElements;
                    } else {
                        self.state = State::Elements;
                    }
                }
                Ok(Token::DtdStart(_, _)) => {
                    self.state = State::Dtd;
                }
                Ok(Token::DtdEnd) => {
                    self.state = State::Document;
                }
                Err(_) => {
                    self.stream.jump_to_end();
                    self.state = State::Finished;
                }
                _ => {}
            }
        }

        t
    }
}

impl<'a> Tokenizer<'a> {
    fn parse_next_impl(s: &mut Stream<'a>, state: State) -> Option<Result<Token<'a>>> {
        if s.at_end() {
            return None;
        }

        macro_rules! parse_token_type {
            () => ({
                match Self::parse_token_type(s, state) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                }
            })
        }

        let start = s.pos();

        macro_rules! gen_err {
            ($token_type:expr) => ({
                let pos = s.gen_error_pos_from(start);
                return Some(Err(ErrorKind::UnexpectedToken($token_type, pos).into()));
            })
        }

        let t = match state {
            State::Document => {
                let token_type = parse_token_type!();
                match token_type {
                    TokenType::XMLDecl => {
                        Self::parse_declaration(s)
                    }
                    TokenType::Comment => {
                        Self::parse_comment(s)
                    }
                    TokenType::PI => {
                        Self::parse_pi(s)
                    }
                    TokenType::DoctypeDecl => {
                        Self::parse_doctype(s)
                    }
                    TokenType::ElementStart => {
                        Self::parse_element_start(s)
                    }
                    TokenType::Whitespace => {
                        s.skip_spaces();
                        return Self::parse_next_impl(s, state);
                    }
                    _ => {
                        gen_err!(token_type);
                    }
                }
            }
            State::Dtd => {
                let token_type = parse_token_type!();
                match token_type {
                      TokenType::ElementDecl
                    | TokenType::NotationDecl
                    | TokenType::AttlistDecl => {
                        if let Err(e) = Self::consume_decl(s) {
                            return Some(Err(e));
                        }

                        return Self::parse_next_impl(s, state);
                    }
                    TokenType::EntityDecl => {
                        Self::parse_entity_decl(s)
                    }
                    TokenType::Comment => {
                        Self::parse_comment(s)
                    }
                    TokenType::PI => {
                        Self::parse_pi(s)
                    }
                    TokenType::DoctypeEnd => {
                        Ok(Token::DtdEnd)
                    }
                    TokenType::Whitespace => {
                        s.skip_spaces();
                        return Self::parse_next_impl(s, state);
                    }
                    _ => {
                        gen_err!(token_type);
                    }
                }
            }
            State::Elements => {
                let token_type = parse_token_type!();

                match token_type {
                    TokenType::ElementStart => {
                        Self::parse_element_start(s)
                    }
                    TokenType::ElementClose => {
                        Self::parse_close_element(s)
                    }
                    TokenType::CDSect => {
                        Self::parse_cdata(s)
                    }
                    TokenType::PI => {
                        Self::parse_pi(s)
                    }
                    TokenType::Comment => {
                        Self::parse_comment(s)
                    }
                    TokenType::CharData => {
                        Self::parse_text(s)
                    }
                    _ => {
                        gen_err!(token_type);
                    }
                }
            }
            State::Attributes => {
                Self::consume_attribute(s).chain_err(|| {
                    ErrorKind::InvalidToken(TokenType::Attribute, s.gen_error_pos_from(start))
                })
            }
            State::AfterElements => {
                let token_type = parse_token_type!();
                match token_type {
                    TokenType::Comment => {
                        Self::parse_comment(s)
                    }
                    TokenType::PI => {
                        Self::parse_pi(s)
                    }
                    TokenType::Whitespace => {
                        s.skip_spaces();
                        return Self::parse_next_impl(s, state);
                    }
                    _ => {
                        gen_err!(token_type);
                    }
                }
            }
            State::Finished => {
                return None;
            }
        };

        Some(t)
    }

    fn parse_token_type(s: &mut Stream, state: State) -> Result<TokenType> {
        let start = s.pos();
        let c1 = s.curr_byte()?;

        let t = match c1 {
            0xEF => {
                // Skip BOM.
                s.advance(3);
                Self::parse_token_type(s, state)?
            }
            b'<' => {
                s.advance(1);

                let c2 = s.curr_byte()?;
                match c2 {
                    b'?' => {
                        // TODO: technically, we should check for any whitespace
                        if s.starts_with(b"?xml ") {
                            s.advance(5);
                            TokenType::XMLDecl
                        } else {
                            s.advance(1);
                            TokenType::PI
                        }
                    }
                    b'!' => {
                        s.advance(1);

                        let c3 = s.curr_byte()?;
                        match c3 {
                            b'-' if s.starts_with(b"--") => {
                                s.advance(2);
                                TokenType::Comment
                            }
                            b'D' if s.starts_with(b"DOCTYPE") => {
                                s.advance(7);
                                TokenType::DoctypeDecl
                            }
                            b'E' if s.starts_with(b"ELEMENT") => {
                                s.advance(7);
                                TokenType::ElementDecl
                            }
                            b'A' if s.starts_with(b"ATTLIST") => {
                                s.advance(7);
                                TokenType::AttlistDecl
                            }
                            b'E' if s.starts_with(b"ENTITY") => {
                                s.advance(6);
                                TokenType::EntityDecl
                            }
                            b'N' if s.starts_with(b"NOTATION") => {
                                s.advance(8);
                                TokenType::NotationDecl
                            }
                            b'[' if s.starts_with(b"[CDATA[") => {
                                s.advance(7);
                                TokenType::CDSect
                            }
                            _ => {
                                let pos = s.gen_error_pos_from(start);
                                return Err(ErrorKind::UnknownToken(pos).into());
                            }
                        }
                    }
                    b'/' => {
                        s.advance(1);
                        TokenType::ElementClose
                    }
                    _ => {
                        TokenType::ElementStart
                    }
                }
            }
            b']' if s.starts_with(b"]>") => {
                s.advance(2);
                TokenType::DoctypeEnd
            }
            _ => {
                match state {
                    State::Document | State::AfterElements | State::Dtd => {
                        if s.starts_with_space() {
                            TokenType::Whitespace
                        } else {
                            let pos = s.gen_error_pos_from(start);
                            return Err(ErrorKind::UnknownToken(pos).into());
                        }
                    }
                    State::Elements => {
                        TokenType::CharData
                    }
                    _ => {
                        let pos = s.gen_error_pos_from(start);
                        return Err(ErrorKind::UnknownToken(pos).into());
                    }
                }
            }
        };

        Ok(t)
    }

    fn parse_declaration(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 6;

        Self::parse_declaration_impl(s).chain_err(|| {
            ErrorKind::InvalidToken(TokenType::XMLDecl, s.gen_error_pos_from(start))
        })
    }

    fn parse_declaration_impl(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let version = Self::parse_version_info(s)?;
        let encoding = Self::parse_encoding_decl(s)?;
        let standalone = Self::parse_standalone(s)?;

        s.skip_ascii_spaces();
        s.skip_string(b"?>")?;

        Ok(Token::Declaration(version, encoding, standalone))
    }

    fn parse_version_info(s: &mut Stream<'a>) -> Result<StrSpan<'a>> {
        s.skip_ascii_spaces();
        s.skip_string(b"version")?;
        s.consume_eq()?;
        s.consume_quote()?;

        let start = s.pos();
        s.skip_string(b"1.")?;
        s.skip_bytes(|_, c| c.is_xml_digit());
        let ver = s.slice_back(start);

        s.consume_quote()?;

        if ver.to_str() != "1.0" {
            warn!("Only XML 1.0 is supported.");
        }

        Ok(ver)
    }

    // S 'encoding' Eq ('"' EncName '"' | "'" EncName "'" )
    fn parse_encoding_decl(s: &mut Stream<'a>) -> Result<Option<StrSpan<'a>>> {
        s.skip_ascii_spaces();

        if s.skip_string(b"encoding").is_err() {
            return Ok(None);
        }

        s.consume_eq()?;
        s.consume_quote()?;
        let name = Self::parse_encoding_name(s)?;
        s.consume_quote()?;

        // TODO: should we keep it?
        // match name.slice() {
        //     "UTF-8" | "utf-8" => {}
        //     _ => {
        //         // It's not an error, since we already know that input data
        //         // is a valid UTF-8 string.
        //         // Probably, the input data has non-UTF-8 encoding, but do not
        //         // use any of non-Latin-1 characters.
        //         warn!("Unsupported encoding: {}", name);
        //     }
        // }

        Ok(Some(name))
    }

    // [A-Za-z] ([A-Za-z0-9._] | '-')*
    fn parse_encoding_name(s: &mut Stream<'a>) -> Result<StrSpan<'a>> {
        // if !s.at_end() {
        //     let c = s.curr_byte_unchecked();
        //     if !Stream::is_letter(c) {
        //         return Err(XmlError::new(ErrorKind::String("Invalid encoding name".into()),
        //                                  Some(s.gen_error_pos())));
        //     }
        // }

        Ok(s.consume_bytes(|_, c| {
               c.is_xml_letter()
            || c.is_xml_digit()
            || c == b'.'
            || c == b'-'
            || c == b'_'
        }))
    }

    // S 'standalone' Eq (("'" ('yes' | 'no') "'") | ('"' ('yes' | 'no') '"'))
    fn parse_standalone(s: &mut Stream<'a>) -> Result<Option<StrSpan<'a>>> {
        s.skip_ascii_spaces();

        if s.skip_string(b"standalone").is_err() {
            return Ok(None);
        }

        s.consume_eq()?;
        s.consume_quote()?;

        let start = s.pos();
        let value = s.consume_name()?;

        match value.to_str() {
            "yes" | "no" => {}
            _ => {
                let c = value.to_str().chars().next().unwrap();
                let pos = s.gen_error_pos_from(start);
                let kind = StreamErrorKind::InvalidChar(c, "yn".into(), pos);
                return Err(StreamError::from(kind).into());
            }
        }

        s.consume_quote()?;

        Ok(Some(value))
    }

    // '<!--' ((Char - '-') | ('-' (Char - '-')))* '-->'
    fn parse_comment(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 4;

        let text = s.consume_chars(|s, c| {
            if c == '-' && s.starts_with(b"-->") {
                return false;
            }

            if !c.is_xml_char() {
                return false;
            }

            true
        });

        if text.to_str().contains("--") {
            let pos = s.gen_error_pos_from(start);
            return Err(ErrorKind::InvalidToken(TokenType::Comment, pos).into());
        }

        if s.skip_string(b"-->").is_err() {
            let pos = s.gen_error_pos_from(start);
            return Err(ErrorKind::InvalidToken(TokenType::Comment, pos).into());
        }

        // if text.slice().ends_with("-") {
        //     warn!("Comment should not end with '--->'. Last '-' is removed.");
        //     text = text.slice_region(0, text.len() - 1);
        // }

        Ok(Token::Comment(text))
    }

    fn parse_pi(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 2;

        Self::parse_pi_impl(s).chain_err(|| {
            ErrorKind::InvalidToken(TokenType::PI, s.gen_error_pos_from(start))
        })
    }

    // PI       ::= '<?' PITarget (S (Char* - (Char* '?>' Char*)))? '?>'
    // PITarget ::= Name - (('X' | 'x') ('M' | 'm') ('L' | 'l'))
    fn parse_pi_impl(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let target = s.consume_name()?;

        s.skip_spaces();

        let content = s.consume_chars(|s, c| {
            if c == '?' && s.starts_with(b"?>") {
                return false;
            }

            if !c.is_xml_char() {
                return false;
            }

            true
        });

        let content = if !content.is_empty() {
            Some(content)
        } else {
            None
        };

        s.skip_string(b"?>")?;

        Ok(Token::ProcessingInstruction(target, content))
    }

    fn parse_doctype(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 9;

        Self::parse_doctype_impl(s).chain_err(|| {
            ErrorKind::InvalidToken(TokenType::DoctypeDecl, s.gen_error_pos_from(start))
        })
    }

    // doctypedecl ::= '<!DOCTYPE' S Name (S ExternalID)? S? ('[' intSubset ']' S?)? '>'
    fn parse_doctype_impl(s: &mut Stream<'a>) -> Result<Token<'a>> {
        s.consume_spaces()?;
        let name = s.consume_name()?;
        s.skip_spaces();

        let id = Self::parse_external_id(s)?;
        s.skip_spaces();

        let c = s.consume_either(&[b'[', b'>'])?;
        if c == b'[' {
            Ok(Token::DtdStart(name, id))
        } else {
            Ok(Token::EmptyDtd(name, id))
        }
    }

    // ExternalID ::= 'SYSTEM' S SystemLiteral | 'PUBLIC' S PubidLiteral S SystemLiteral
    fn parse_external_id(s: &mut Stream<'a>) -> Result<Option<ExternalId<'a>>> {
        let v = if s.starts_with(b"SYSTEM") || s.starts_with(b"PUBLIC") {
            let start = s.pos();
            s.advance(6);
            let id = s.slice_back(start);

            s.consume_spaces()?;
            let quote = s.consume_quote()?;
            let literal1 = s.consume_bytes(|_, c| c != quote);
            s.consume_byte(quote)?;

            let v = if id.to_str() == "SYSTEM" {
                ExternalId::System(literal1)
            } else {
                s.consume_spaces()?;
                let quote = s.consume_quote()?;
                let literal2 = s.consume_bytes(|_, c| c != quote);
                s.consume_byte(quote)?;

                ExternalId::Public(literal1, literal2)
            };

            Some(v)
        } else {
            None
        };

        Ok(v)
    }

    fn parse_entity_decl(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 8;

        Self::parse_entity_decl_impl(s).chain_err(|| {
            ErrorKind::InvalidToken(TokenType::EntityDecl, s.gen_error_pos_from(start))
        })
    }

    // EntityDecl  ::= GEDecl | PEDecl
    // GEDecl      ::= '<!ENTITY' S Name S EntityDef S? '>'
    // PEDecl      ::= '<!ENTITY' S '%' S Name S PEDef S? '>'
    fn parse_entity_decl_impl(s: &mut Stream<'a>) -> Result<Token<'a>> {
        s.consume_spaces()?;

        let is_ge = if s.curr_byte()? == b'%' {
            s.consume_byte(b'%')?;
            s.consume_spaces()?;

            false
        } else {
            true
        };

        let name = s.consume_name()?;
        s.consume_spaces()?;
        let def = Self::parse_entity_def(s, is_ge)?;
        s.skip_spaces();
        s.consume_byte(b'>')?;

        Ok(Token::EntityDeclaration(name, def))
    }

    // EntityDef   ::= EntityValue | (ExternalID NDataDecl?)
    // PEDef       ::= EntityValue | ExternalID
    // EntityValue ::= '"' ([^%&"] | PEReference | Reference)* '"' |  "'" ([^%&']
    //                             | PEReference | Reference)* "'"
    // ExternalID  ::= 'SYSTEM' S SystemLiteral | 'PUBLIC' S PubidLiteral S SystemLiteral
    // NDataDecl   ::= S 'NDATA' S Name
    fn parse_entity_def(s: &mut Stream<'a>, is_ge: bool) -> Result<EntityDefinition<'a>> {
        let c = s.curr_byte()?;
        match c {
            b'"' | b'\'' => {
                let quote = s.consume_quote()?;
                let value = s.consume_bytes(|_, c| c != quote);
                s.consume_byte(quote)?;

                Ok(EntityDefinition::EntityValue(value))
            }
            b'S' | b'P' => {
                if let Some(id) = Self::parse_external_id(s)? {
                    if is_ge {
                        s.skip_spaces();
                        if s.starts_with(b"NDATA") {
                            s.skip_string(b"NDATA")?;
                            s.consume_spaces()?;
                            s.skip_name()?;
                            warn!("NDataDecl is not supported.")
                        }
                    }

                    Ok(EntityDefinition::ExternalId(id))
                } else {
                    Err("invalid ExternalID".into())
                }
            }
            _ => {
                let pos = s.gen_error_pos();
                let kind = StreamErrorKind::InvalidChar(c as char, "\"'SP".into(), pos);
                Err(StreamError::from(kind).into())
            }
        }
    }

    fn consume_decl(s: &mut Stream) -> Result<()> {
        s.consume_spaces()?;
        s.skip_bytes(|_, c| c != b'>');
        s.consume_byte(b'>')?;

        // warn!("'{}' skipped.", s.slice_back(start));

        Ok(())
    }

    // CDSect  ::= CDStart CData CDEnd
    // CDStart ::= '<![CDATA['
    // CData   ::= (Char* - (Char* ']]>' Char*))
    // CDEnd   ::= ']]>'
    fn parse_cdata(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 9;

        let text = s.consume_bytes(|s, c| {
            !(c == b']' && s.starts_with(b"]]>"))
        });

        s.skip_string(b"]]>").chain_err(|| {
            ErrorKind::InvalidToken(TokenType::CDSect, s.gen_error_pos_from(start))
        })?;

        Ok(Token::Cdata(text))
    }

    fn parse_element_start(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 1;

        Self::parse_element_start_impl(s).chain_err(|| {
            ErrorKind::InvalidToken(TokenType::ElementStart, s.gen_error_pos_from(start))
        })
    }

    // '<' Name (S Attribute)* S? '>'
    fn parse_element_start_impl(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let tag_name = s.consume_name()?;
        Ok(Token::ElementStart(tag_name))
    }

    fn parse_close_element(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let start = s.pos() - 2;

        Self::parse_close_element_impl(s).chain_err(|| {
            ErrorKind::InvalidToken(TokenType::ElementClose, s.gen_error_pos_from(start))
        })
    }

    // '</' Name S? '>'
    fn parse_close_element_impl(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let tag_name = s.consume_name()?;
        s.skip_ascii_spaces();
        s.consume_byte(b'>')?;

        Ok(Token::ElementEnd(ElementEnd::Close(tag_name)))
    }

    // Name Eq AttValue
    fn consume_attribute(s: &mut Stream<'a>) -> Result<Token<'a>> {
        s.skip_ascii_spaces();

        if let Some(c) = s.get_curr_byte() {
            match c {
                b'/' => {
                    s.advance(1);
                    s.consume_byte(b'>')?;
                    return Ok(Token::ElementEnd(ElementEnd::Empty));
                }
                b'>' => {
                    s.advance(1);
                    return Ok(Token::ElementEnd(ElementEnd::Open));
                }
                _ => {}
            }
        }

        let name = s.consume_name()?;
        s.consume_eq()?;
        let quote = s.consume_quote()?;
        let value = s.consume_bytes(|_, c| c != quote);

        // if s.curr_byte()? == b'<' {
        //     let kind = StreamErrorKind::InvalidChar('<', "Char".into(), s.gen_error_pos());
        //     return Err(StreamError::from(kind).into());
        // }

        s.consume_byte(quote)?;
        s.skip_ascii_spaces();

        Ok(Token::Attribute(name, value))
    }

    fn parse_text(s: &mut Stream<'a>) -> Result<Token<'a>> {
        let text = s.consume_bytes(|_, c| c != b'<');

        let mut ts = Stream::from_span(text);
        // TODO: optimize
        ts.skip_spaces();
        if ts.at_end() {
            Ok(Token::Whitespaces(text))
        } else {
            Ok(Token::Text(text))
        }
    }
}