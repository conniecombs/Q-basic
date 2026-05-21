#![allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    StringLit(String),
    Identifier(String),
    Print,
    Let,
    Input,
    If,
    Then,
    Else,
    ElseIf,
    EndIf,
    For,
    To,
    Step,
    Next,
    While,
    Wend,
    Do,
    Loop,
    Until,
    Goto,
    Gosub,
    Return,
    End,
    Rem,
    Dim,
    Data,
    Read,
    Restore,
    As,
    Integer,
    Single,
    Double,
    Long,
    StringKw,
    Sub,
    Function,
    EndSub,
    EndFunction,
    Call,
    Exit,
    ByRef,
    ByVal,
    Const,
    Type,
    EndType,
    Select,
    Case,
    EndSelect,
    Is,
    Open,
    Close,
    Output,
    Append,
    Random,
    Hash,
    Get,
    Put,
    LenKw,
    Screen,
    Line,
    Circle,
    Pset,
    Color,
    Cls,
    Window,
    Paint,
    Mod,
    And,
    Or,
    Not,
    Xor,
    Randomize,
    Sleep,
    Equals,
    Plus,
    Minus,
    Star,
    Slash,
    Backslash,
    Caret,
    LParen,
    RParen,
    Comma,
    Semicolon,
    Colon,
    Dot,
    NotEqual,
    LessEqual,
    GreaterEqual,
    Less,
    Greater,
    LineNumber(u32),
    Label(String),
    Newline,
    Eof,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    for raw_line in input.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            tokens.push(Token::Newline);
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        let mut at_line_start = true;
        while i < chars.len() {
            let c = chars[i];
            if c == ' ' || c == '\t' {
                i += 1;
                continue;
            }
            if at_line_start && c.is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n: u32 = s.parse().map_err(|e| format!("Bad line number: {}", e))?;
                tokens.push(Token::LineNumber(n));
                at_line_start = false;
                continue;
            }
            at_line_start = false;
            if c == '\'' {
                break;
            }
            if c == '#' {
                tokens.push(Token::Hash);
                i += 1;
                continue;
            }
            if c.is_ascii_digit()
                || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
            {
                let start = i;
                let mut saw_dot = false;
                while i < chars.len()
                    && (chars[i].is_ascii_digit() || (chars[i] == '.' && !saw_dot))
                {
                    if chars[i] == '.' {
                        saw_dot = true;
                    }
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n: f64 = s
                    .parse()
                    .map_err(|e| format!("Bad number '{}': {}", s, e))?;
                tokens.push(Token::Number(n));
                continue;
            }
            if c == '"' {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("Unterminated string".to_string());
                }
                let s: String = chars[start..i].iter().collect();
                i += 1;
                tokens.push(Token::StringLit(s));
                continue;
            }
            if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                if i < chars.len()
                    && (chars[i] == '$'
                        || chars[i] == '%'
                        || chars[i] == '!'
                        || chars[i] == '#'
                        || chars[i] == '&')
                {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let upper = word.to_uppercase();
                if upper == "REM" {
                    break;
                }
                let is_label_candidate = i < chars.len() && chars[i] == ':' && !is_keyword(&upper);
                if is_label_candidate {
                    i += 1;
                    tokens.push(Token::Label(upper)); // OPTIMIZATION: Stored as uppercase
                    continue;
                }
                match upper.as_str() {
                    "PRINT" => tokens.push(Token::Print),
                    "LET" => tokens.push(Token::Let),
                    "INPUT" => tokens.push(Token::Input),
                    "IF" => tokens.push(Token::If),
                    "THEN" => tokens.push(Token::Then),
                    "ELSE" => tokens.push(Token::Else),
                    "ELSEIF" => tokens.push(Token::ElseIf),
                    "ENDIF" => tokens.push(Token::EndIf),
                    "FOR" => tokens.push(Token::For),
                    "TO" => tokens.push(Token::To),
                    "STEP" => tokens.push(Token::Step),
                    "NEXT" => tokens.push(Token::Next),
                    "WHILE" => tokens.push(Token::While),
                    "WEND" => tokens.push(Token::Wend),
                    "DO" => tokens.push(Token::Do),
                    "LOOP" => tokens.push(Token::Loop),
                    "UNTIL" => tokens.push(Token::Until),
                    "GOTO" => tokens.push(Token::Goto),
                    "GOSUB" => tokens.push(Token::Gosub),
                    "RETURN" => tokens.push(Token::Return),
                    "DIM" => tokens.push(Token::Dim),
                    "DATA" => tokens.push(Token::Data),
                    "READ" => tokens.push(Token::Read),
                    "RESTORE" => tokens.push(Token::Restore),
                    "AS" => tokens.push(Token::As),
                    "INTEGER" => tokens.push(Token::Integer),
                    "LONG" => tokens.push(Token::Long),
                    "SINGLE" => tokens.push(Token::Single),
                    "DOUBLE" => tokens.push(Token::Double),
                    "STRING" => tokens.push(Token::StringKw),
                    "SUB" => tokens.push(Token::Sub),
                    "FUNCTION" => tokens.push(Token::Function),
                    "CALL" => tokens.push(Token::Call),
                    "EXIT" => tokens.push(Token::Exit),
                    "BYREF" => tokens.push(Token::ByRef),
                    "BYVAL" => tokens.push(Token::ByVal),
                    "CONST" => tokens.push(Token::Const),
                    "TYPE" => tokens.push(Token::Type),
                    "SELECT" => tokens.push(Token::Select),
                    "CASE" => tokens.push(Token::Case),
                    "IS" => tokens.push(Token::Is),
                    "OPEN" => tokens.push(Token::Open),
                    "CLOSE" => tokens.push(Token::Close),
                    "OUTPUT" => tokens.push(Token::Output),
                    "APPEND" => tokens.push(Token::Append),
                    "RANDOM" => tokens.push(Token::Random),
                    "GET" => tokens.push(Token::Get),
                    "PUT" => tokens.push(Token::Put),
                    "LEN" => tokens.push(Token::LenKw),
                    "SCREEN" => tokens.push(Token::Screen),
                    "LINE" => tokens.push(Token::Line),
                    "CIRCLE" => tokens.push(Token::Circle),
                    "PSET" => tokens.push(Token::Pset),
                    "COLOR" => tokens.push(Token::Color),
                    "CLS" => tokens.push(Token::Cls),
                    "WINDOW" => tokens.push(Token::Window),
                    "PAINT" => tokens.push(Token::Paint),
                    "MOD" => tokens.push(Token::Mod),
                    "AND" => tokens.push(Token::And),
                    "OR" => tokens.push(Token::Or),
                    "NOT" => tokens.push(Token::Not),
                    "XOR" => tokens.push(Token::Xor),
                    "RANDOMIZE" => tokens.push(Token::Randomize),
                    "SLEEP" => tokens.push(Token::Sleep),
                    "END" => {
                        let mut j = i;
                        while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                            j += 1;
                        }
                        let rest: String = chars[j..].iter().collect();
                        let rest_upper = rest.to_uppercase();
                        let after_ok = |off: usize| {
                            j + off >= chars.len() || !chars[j + off].is_ascii_alphanumeric()
                        };
                        if rest_upper.starts_with("IF") && after_ok(2) {
                            i = j + 2;
                            tokens.push(Token::EndIf);
                        } else if rest_upper.starts_with("SUB") && after_ok(3) {
                            i = j + 3;
                            tokens.push(Token::EndSub);
                        } else if rest_upper.starts_with("FUNCTION") && after_ok(8) {
                            i = j + 8;
                            tokens.push(Token::EndFunction);
                        } else if rest_upper.starts_with("TYPE") && after_ok(4) {
                            i = j + 4;
                            tokens.push(Token::EndType);
                        } else if rest_upper.starts_with("SELECT") && after_ok(6) {
                            i = j + 6;
                            tokens.push(Token::EndSelect);
                        } else {
                            tokens.push(Token::End);
                        }
                    }
                    _ => tokens.push(Token::Identifier(upper)), // OPTIMIZATION: Stored as uppercase
                }
                continue;
            }
            match c {
                '=' => {
                    tokens.push(Token::Equals);
                    i += 1;
                }
                '+' => {
                    tokens.push(Token::Plus);
                    i += 1;
                }
                '-' => {
                    tokens.push(Token::Minus);
                    i += 1;
                }
                '*' => {
                    tokens.push(Token::Star);
                    i += 1;
                }
                '/' => {
                    tokens.push(Token::Slash);
                    i += 1;
                }
                '\\' => {
                    tokens.push(Token::Backslash);
                    i += 1;
                }
                '^' => {
                    tokens.push(Token::Caret);
                    i += 1;
                }
                '(' => {
                    tokens.push(Token::LParen);
                    i += 1;
                }
                ')' => {
                    tokens.push(Token::RParen);
                    i += 1;
                }
                ',' => {
                    tokens.push(Token::Comma);
                    i += 1;
                }
                ';' => {
                    tokens.push(Token::Semicolon);
                    i += 1;
                }
                ':' => {
                    tokens.push(Token::Colon);
                    i += 1;
                }
                '.' => {
                    tokens.push(Token::Dot);
                    i += 1;
                }
                '<' => {
                    if i + 1 < chars.len() && chars[i + 1] == '=' {
                        tokens.push(Token::LessEqual);
                        i += 2;
                    } else if i + 1 < chars.len() && chars[i + 1] == '>' {
                        tokens.push(Token::NotEqual);
                        i += 2;
                    } else {
                        tokens.push(Token::Less);
                        i += 1;
                    }
                }
                '>' => {
                    if i + 1 < chars.len() && chars[i + 1] == '=' {
                        tokens.push(Token::GreaterEqual);
                        i += 2;
                    } else {
                        tokens.push(Token::Greater);
                        i += 1;
                    }
                }
                _ => return Err(format!("Unexpected character '{}'", c)),
            }
        }
        tokens.push(Token::Newline);
    }
    tokens.push(Token::Eof);
    Ok(tokens)
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "PRINT"
            | "LET"
            | "INPUT"
            | "IF"
            | "THEN"
            | "ELSE"
            | "ELSEIF"
            | "ENDIF"
            | "FOR"
            | "TO"
            | "STEP"
            | "NEXT"
            | "WHILE"
            | "WEND"
            | "DO"
            | "LOOP"
            | "UNTIL"
            | "GOTO"
            | "GOSUB"
            | "RETURN"
            | "END"
            | "REM"
            | "DIM"
            | "DATA"
            | "READ"
            | "RESTORE"
            | "AS"
            | "INTEGER"
            | "LONG"
            | "SINGLE"
            | "DOUBLE"
            | "STRING"
            | "SUB"
            | "FUNCTION"
            | "CALL"
            | "EXIT"
            | "BYREF"
            | "BYVAL"
            | "CONST"
            | "TYPE"
            | "SELECT"
            | "CASE"
            | "IS"
            | "OPEN"
            | "CLOSE"
            | "OUTPUT"
            | "APPEND"
            | "RANDOM"
            | "GET"
            | "PUT"
            | "LEN"
            | "SCREEN"
            | "LINE"
            | "CIRCLE"
            | "PSET"
            | "COLOR"
            | "CLS"
            | "WINDOW"
            | "PAINT"
            | "MOD"
            | "AND"
            | "OR"
            | "NOT"
            | "XOR"
            | "RANDOMIZE"
            | "SLEEP"
    )
}
