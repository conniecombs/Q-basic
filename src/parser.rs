#![allow(dead_code)]
use crate::lexer::Token;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Integer,
    Long,
    Single,
    Double,
    Str,
    UserType(String),
    Auto,
}

impl VarType {
    pub fn is_string(&self) -> bool {
        matches!(self, VarType::Str)
    }
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            VarType::Integer | VarType::Long | VarType::Single | VarType::Double
        )
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    StringLit(String),
    Variable(String),
    FieldAccess(Box<Expr>, String),
    ArrayOrCall(String, Vec<Expr>),
    BinaryOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryMinus(Box<Expr>),
    Not(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IntDiv,
    Mod,
    Pow,
    Eq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone)]
pub enum PrintItem {
    Expr(Expr),
    Semicolon,
    Comma,
}

#[derive(Debug, Clone)]
pub enum LValue {
    Var(String),
    Index(String, Vec<Expr>),
    Field(Box<LValue>, String),
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub vtype: VarType,
    pub by_ref: bool,
    pub is_array: bool,
}

#[derive(Debug, Clone)]
pub enum CaseClause {
    Value(Expr),
    Range(Expr, Expr),
    Compare(BinOp, Expr),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Print(Option<u32>, Vec<PrintItem>),
    Let(LValue, Expr),
    Input(Option<u32>, Option<String>, Vec<LValue>),
    If {
        cond: Expr,
        then_branch: Vec<Stmt>,
        elseifs: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Vec<Stmt>,
    },
    SelectCase {
        expr: Expr,
        cases: Vec<(Vec<CaseClause>, Vec<Stmt>)>,
        else_branch: Vec<Stmt>,
    },
    For {
        var: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    DoLoop {
        cond: Option<(Expr, bool, bool)>,
        body: Vec<Stmt>,
    },
    Goto(String),
    Gosub(String),
    Return,
    End,
    Exit(String),
    Label(String),
    LineNumber(u32),
    Dim(Vec<DimDecl>),
    Const(String, Expr),
    TypeDef(String, Vec<(String, VarType)>),
    SubDef {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    FuncDef {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        ret_type: VarType,
    },
    CallSub(String, Vec<Expr>),
    Open {
        filename: Expr,
        mode: FileMode,
        handle: u32,
        record_len: Option<Expr>,
    },
    Close(Vec<u32>),
    Get {
        handle: u32,
        record: Option<Expr>,
        var: LValue,
    },
    Put {
        handle: u32,
        record: Option<Expr>,
        var: LValue,
    },
    Screen(Expr),
    Window(Option<Expr>),
    Cls,
    Color(Option<Expr>, Option<Expr>),
    Pset(Expr, Expr, Option<Expr>),
    Line(Option<(Expr, Expr)>, Expr, Expr, Option<Expr>, bool, bool),
    Circle(Expr, Expr, Expr, Option<Expr>),
    Paint(Expr, Expr, Option<Expr>, Option<Expr>),
    Sleep(Option<Expr>),
}

#[derive(Debug, Clone)]
pub struct DimDecl {
    pub name: String,
    pub dims: Vec<Expr>,
    pub vtype: VarType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileMode {
    Input,
    Output,
    Append,
    Random,
}

pub struct Program {
    pub stmts: Vec<Stmt>,
}

pub fn parse(tokens: Vec<Token>) -> Result<Program, String> {
    let mut p = Parser {
        tokens,
        pos: 0,
        var_types: HashMap::new(),
        type_defs: HashMap::new(),
        constants: HashMap::new(),
        sub_sigs: HashMap::new(),
    };
    Ok(Program {
        stmts: p.parse_program()?,
    })
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    var_types: HashMap<String, VarType>,
    type_defs: HashMap<String, Vec<(String, VarType)>>,
    constants: HashMap<String, VarType>,
    sub_sigs: HashMap<String, (Vec<Param>, bool, VarType)>,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }
    fn peek_at(&self, off: usize) -> &Token {
        if self.pos + off < self.tokens.len() {
            &self.tokens[self.pos + off]
        } else {
            &Token::Eof
        }
    }
    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn check(&self, t: &Token) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(t)
    }
    fn matches(&mut self, t: &Token) -> bool {
        if self.check(t) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) {
            self.advance();
        }
    }
    fn parse_program(&mut self) -> Result<Vec<Stmt>, String> {
        self.prescan_signatures()?;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Eof) {
                break;
            }
            stmts.extend(self.parse_line()?);
        }
        Ok(stmts)
    }
    fn prescan_signatures(&mut self) -> Result<(), String> {
        let saved_pos = self.pos;
        let mut i = 0;
        while i < self.tokens.len() {
            match &self.tokens[i] {
                Token::Sub | Token::Function => {
                    let is_func = matches!(self.tokens[i], Token::Function);
                    let mut j = i + 1;
                    let name = if let Token::Identifier(n) = &self.tokens[j] {
                        j += 1;
                        n.clone()
                    } else {
                        i += 1;
                        continue;
                    };
                    let mut params: Vec<Param> = Vec::new();
                    if matches!(self.tokens[j], Token::LParen) {
                        j += 1;
                        while !matches!(self.tokens[j], Token::RParen | Token::Eof) {
                            let by_ref = if matches!(self.tokens[j], Token::ByRef) {
                                j += 1;
                                true
                            } else if matches!(self.tokens[j], Token::ByVal) {
                                j += 1;
                                false
                            } else {
                                true
                            };
                            let pname = if let Token::Identifier(n) = &self.tokens[j] {
                                j += 1;
                                n.clone()
                            } else {
                                break;
                            };
                            let mut is_array = false;
                            if matches!(self.tokens[j], Token::LParen) {
                                j += 1;
                                if matches!(self.tokens[j], Token::RParen) {
                                    j += 1;
                                    is_array = true;
                                }
                            }
                            let vt = if matches!(self.tokens[j], Token::As) {
                                j += 1;
                                let t = match &self.tokens[j] {
                                    Token::Integer => VarType::Integer,
                                    Token::Long => VarType::Long,
                                    Token::Single => VarType::Single,
                                    Token::Double => VarType::Double,
                                    Token::StringKw => VarType::Str,
                                    Token::Identifier(s) => VarType::UserType(s.clone()),
                                    _ => VarType::Auto,
                                };
                                j += 1;
                                t
                            } else {
                                infer_type_from_suffix(&pname)
                            };
                            params.push(Param {
                                name: pname,
                                vtype: vt,
                                by_ref,
                                is_array,
                            });
                            if matches!(self.tokens[j], Token::Comma) {
                                j += 1;
                            }
                        }
                    }
                    let ret_type = if is_func {
                        if matches!(self.tokens[j], Token::As) {
                            j += 1;
                            match &self.tokens[j] {
                                Token::Integer => VarType::Integer,
                                Token::Long => VarType::Long,
                                Token::Single => VarType::Single,
                                Token::Double => VarType::Double,
                                Token::StringKw => VarType::Str,
                                _ => infer_type_from_suffix(&name),
                            }
                        } else {
                            infer_type_from_suffix(&name)
                        }
                    } else {
                        VarType::Auto
                    };
                    self.sub_sigs
                        .insert(name.to_uppercase(), (params, is_func, ret_type));
                    i = j;
                }
                _ => {
                    i += 1;
                }
            }
        }
        self.pos = saved_pos;
        Ok(())
    }
    fn parse_line(&mut self) -> Result<Vec<Stmt>, String> {
        let mut result = Vec::new();
        if let Token::LineNumber(n) = self.peek().clone() {
            self.advance();
            result.push(Stmt::LineNumber(n));
        }
        if let Token::Label(name) = self.peek().clone() {
            self.advance();
            result.push(Stmt::Label(name));
        }
        loop {
            if matches!(self.peek(), Token::Newline | Token::Eof) {
                break;
            }
            let s = self.parse_stmt()?;
            result.push(s);
            if matches!(self.peek(), Token::Colon) {
                self.advance();
                continue;
            }
            break;
        }
        if matches!(self.peek(), Token::Newline) {
            self.advance();
        }
        Ok(result)
    }
    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek().clone() {
            Token::Print => self.parse_print(),
            Token::Let => {
                self.advance();
                self.parse_assignment()
            }
            Token::Identifier(_) => self.parse_id_stmt(),
            Token::Input => self.parse_input(),
            Token::If => self.parse_if(),
            Token::Select => self.parse_select(),
            Token::For => self.parse_for(),
            Token::While => self.parse_while(),
            Token::Do => self.parse_do(),
            Token::Goto => {
                self.advance();
                self.parse_goto_target().map(Stmt::Goto)
            }
            Token::Gosub => {
                self.advance();
                self.parse_goto_target().map(Stmt::Gosub)
            }
            Token::Return => {
                self.advance();
                Ok(Stmt::Return)
            }
            Token::End => {
                self.advance();
                Ok(Stmt::End)
            }
            Token::Exit => self.parse_exit(),
            Token::Dim => self.parse_dim(),
            Token::Const => self.parse_const(),
            Token::Type => self.parse_type_def(),
            Token::Sub => self.parse_sub(),
            Token::Function => self.parse_function(),
            Token::Call => self.parse_call(),
            Token::Open => self.parse_open(),
            Token::Close => self.parse_close(),
            Token::Get => self.parse_get(),
            Token::Put => self.parse_put(),
            Token::Screen => {
                self.advance();
                let e = self.parse_expr()?;
                Ok(Stmt::Screen(e))
            }
            Token::Window => {
                self.advance();
                let arg = if matches!(self.peek(), Token::Newline | Token::Eof | Token::Colon) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                Ok(Stmt::Window(arg))
            }
            Token::Cls => {
                self.advance();
                Ok(Stmt::Cls)
            }
            Token::Color => self.parse_color(),
            Token::Pset => self.parse_pset(),
            Token::Line => self.parse_line_stmt(),
            Token::Circle => self.parse_circle(),
            Token::Paint => self.parse_paint(),
            Token::Sleep => {
                self.advance();
                let arg = if matches!(self.peek(), Token::Newline | Token::Eof | Token::Colon) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                Ok(Stmt::Sleep(arg))
            }
            other => Err(format!("Unexpected token: {:?}", other)),
        }
    }
    fn parse_goto_target(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Number(n) => Ok(format!("{}", n as u32)),
            Token::Identifier(s) => Ok(s),
            t => Err(format!("Expected target, got {:?}", t)),
        }
    }
    fn parse_print(&mut self) -> Result<Stmt, String> {
        self.advance();
        let mut handle = None;
        if matches!(self.peek(), Token::Hash) {
            self.advance();
            if let Token::Number(n) = self.advance() {
                handle = Some(n as u32);
            } else {
                return Err("Expected file number after #".to_string());
            }
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            }
        }
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::Newline
                | Token::Eof
                | Token::Colon
                | Token::Else
                | Token::ElseIf
                | Token::EndIf => break,
                Token::Semicolon => {
                    self.advance();
                    items.push(PrintItem::Semicolon);
                }
                Token::Comma => {
                    self.advance();
                    items.push(PrintItem::Comma);
                }
                _ => {
                    let e = self.parse_expr()?;
                    items.push(PrintItem::Expr(e));
                }
            }
        }
        Ok(Stmt::Print(handle, items))
    }
    fn parse_id_stmt(&mut self) -> Result<Stmt, String> {
        let name = if let Token::Identifier(n) = self.advance() {
            n
        } else {
            unreachable!()
        };
        if matches!(self.peek(), Token::Dot) {
            let mut lv = LValue::Var(name);
            while matches!(self.peek(), Token::Dot) {
                self.advance();
                let f = match self.advance() {
                    Token::Identifier(s) => s,
                    t => return Err(format!("Expected field name, got {:?}", t)),
                };
                lv = LValue::Field(Box::new(lv), f);
            }
            if !self.matches(&Token::Equals) {
                return Err("Expected '=' after field access".to_string());
            }
            let e = self.parse_expr()?;
            self.check_assignment_types(&lv, &e)?;
            return Ok(Stmt::Let(lv, e));
        }
        if matches!(self.peek(), Token::Equals) {
            self.advance();
            let e = self.parse_expr()?;
            let lv = LValue::Var(name.clone());
            self.declare_var_if_needed(&name);
            self.check_assignment_types(&lv, &e)?;
            return Ok(Stmt::Let(lv, e));
        }
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut args = Vec::new();
            if !matches!(self.peek(), Token::RParen) {
                args.push(self.parse_expr()?);
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
            }
            if !self.matches(&Token::RParen) {
                return Err("Expected ')'".to_string());
            }
            if matches!(self.peek(), Token::Equals) {
                self.advance();
                let e = self.parse_expr()?;
                let lv = LValue::Index(name, args);
                self.check_assignment_types(&lv, &e)?;
                return Ok(Stmt::Let(lv, e));
            }
            return Ok(Stmt::CallSub(name, args));
        }
        if matches!(self.peek(), Token::Newline | Token::Eof | Token::Colon) {
            return Ok(Stmt::CallSub(name, vec![]));
        }
        let mut args = Vec::new();
        args.push(self.parse_expr()?);
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            args.push(self.parse_expr()?);
        }
        Ok(Stmt::CallSub(name, args))
    }
    fn parse_assignment(&mut self) -> Result<Stmt, String> {
        let name = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected variable name, got {:?}", t)),
        };
        let mut lv = if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut args = Vec::new();
            if !matches!(self.peek(), Token::RParen) {
                args.push(self.parse_expr()?);
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
            }
            if !self.matches(&Token::RParen) {
                return Err("Expected ')'".to_string());
            }
            LValue::Index(name.clone(), args)
        } else {
            LValue::Var(name.clone())
        };
        while matches!(self.peek(), Token::Dot) {
            self.advance();
            let f = match self.advance() {
                Token::Identifier(s) => s,
                t => return Err(format!("Expected field, got {:?}", t)),
            };
            lv = LValue::Field(Box::new(lv), f);
        }
        if !self.matches(&Token::Equals) {
            return Err("Expected '=' in assignment".to_string());
        }
        let e = self.parse_expr()?;
        self.declare_var_if_needed(&name);
        self.check_assignment_types(&lv, &e)?;
        Ok(Stmt::Let(lv, e))
    }
    fn parse_input(&mut self) -> Result<Stmt, String> {
        self.advance();
        let mut handle = None;
        if matches!(self.peek(), Token::Hash) {
            self.advance();
            if let Token::Number(n) = self.advance() {
                handle = Some(n as u32);
            } else {
                return Err("Expected file number after #".to_string());
            }
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            }
        }
        let mut prompt = None;
        if let Token::StringLit(s) = self.peek().clone() {
            self.advance();
            prompt = Some(s);
            if matches!(self.peek(), Token::Semicolon | Token::Comma) {
                self.advance();
            }
        }
        let mut vars = Vec::new();
        loop {
            let name = match self.advance() {
                Token::Identifier(n) => n,
                t => return Err(format!("Expected variable in INPUT, got {:?}", t)),
            };
            let mut lv = LValue::Var(name.clone());
            while matches!(self.peek(), Token::Dot) {
                self.advance();
                let f = match self.advance() {
                    Token::Identifier(s) => s,
                    t => return Err(format!("Expected field, got {:?}", t)),
                };
                lv = LValue::Field(Box::new(lv), f);
            }
            self.declare_var_if_needed(&name);
            vars.push(lv);
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(Stmt::Input(handle, prompt, vars))
    }
    fn parse_if(&mut self) -> Result<Stmt, String> {
        self.advance();
        let cond = self.parse_expr()?;
        if !self.matches(&Token::Then) {
            return Err("Expected THEN".to_string());
        }
        let mut then_branch = Vec::new();
        let mut elseifs: Vec<(Expr, Vec<Stmt>)> = Vec::new();
        let mut else_branch = Vec::new();
        if matches!(self.peek(), Token::Newline) {
            self.advance();
            loop {
                self.skip_newlines();
                if matches!(
                    self.peek(),
                    Token::Else | Token::ElseIf | Token::EndIf | Token::Eof
                ) {
                    break;
                }
                then_branch.extend(self.parse_line()?);
            }
            while matches!(self.peek(), Token::ElseIf) {
                self.advance();
                let c = self.parse_expr()?;
                if !self.matches(&Token::Then) {
                    return Err("Expected THEN after ELSEIF".to_string());
                }
                if matches!(self.peek(), Token::Newline) {
                    self.advance();
                }
                let mut block = Vec::new();
                loop {
                    self.skip_newlines();
                    if matches!(
                        self.peek(),
                        Token::Else | Token::ElseIf | Token::EndIf | Token::Eof
                    ) {
                        break;
                    }
                    block.extend(self.parse_line()?);
                }
                elseifs.push((c, block));
            }
            if matches!(self.peek(), Token::Else) {
                self.advance();
                if matches!(self.peek(), Token::Newline) {
                    self.advance();
                }
                loop {
                    self.skip_newlines();
                    if matches!(self.peek(), Token::EndIf | Token::Eof) {
                        break;
                    }
                    else_branch.extend(self.parse_line()?);
                }
            }
            if !self.matches(&Token::EndIf) {
                return Err("Expected END IF".to_string());
            }
        } else {
            if let Token::Number(n) = self.peek().clone() {
                self.advance();
                then_branch.push(Stmt::Goto(format!("{}", n as u32)));
            } else {
                then_branch.push(self.parse_stmt()?);
                while matches!(self.peek(), Token::Colon) {
                    self.advance();
                    if matches!(self.peek(), Token::Else | Token::Newline | Token::Eof) {
                        break;
                    }
                    then_branch.push(self.parse_stmt()?);
                }
            }
            if matches!(self.peek(), Token::Else) {
                self.advance();
                if let Token::Number(n) = self.peek().clone() {
                    self.advance();
                    else_branch.push(Stmt::Goto(format!("{}", n as u32)));
                } else {
                    else_branch.push(self.parse_stmt()?);
                    while matches!(self.peek(), Token::Colon) {
                        self.advance();
                        if matches!(self.peek(), Token::Newline | Token::Eof) {
                            break;
                        }
                        else_branch.push(self.parse_stmt()?);
                    }
                }
            }
        }
        Ok(Stmt::If {
            cond,
            then_branch,
            elseifs,
            else_branch,
        })
    }
    fn parse_select(&mut self) -> Result<Stmt, String> {
        self.advance();
        if !self.matches(&Token::Case) {
            return Err("Expected CASE after SELECT".to_string());
        }
        let expr = self.parse_expr()?;
        self.skip_newlines();
        let mut cases = Vec::new();
        let mut else_branch = Vec::new();
        while matches!(self.peek(), Token::Case) {
            self.advance();
            if matches!(self.peek(), Token::Else) {
                self.advance();
                self.skip_newlines();
                loop {
                    self.skip_newlines();
                    if matches!(self.peek(), Token::Case | Token::EndSelect | Token::Eof) {
                        break;
                    }
                    else_branch.extend(self.parse_line()?);
                }
                continue;
            }
            let mut clauses = Vec::new();
            loop {
                let clause = if matches!(self.peek(), Token::Is) {
                    self.advance();
                    let op = match self.advance() {
                        Token::Equals => BinOp::Eq,
                        Token::NotEqual => BinOp::NotEq,
                        Token::Less => BinOp::Lt,
                        Token::LessEqual => BinOp::Le,
                        Token::Greater => BinOp::Gt,
                        Token::GreaterEqual => BinOp::Ge,
                        t => return Err(format!("Expected comparison after IS, got {:?}", t)),
                    };
                    let e = self.parse_expr()?;
                    CaseClause::Compare(op, e)
                } else {
                    let a = self.parse_expr()?;
                    if matches!(self.peek(), Token::To) {
                        self.advance();
                        let b = self.parse_expr()?;
                        CaseClause::Range(a, b)
                    } else {
                        CaseClause::Value(a)
                    }
                };
                clauses.push(clause);
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.skip_newlines();
            let mut body = Vec::new();
            loop {
                self.skip_newlines();
                if matches!(self.peek(), Token::Case | Token::EndSelect | Token::Eof) {
                    break;
                }
                body.extend(self.parse_line()?);
            }
            cases.push((clauses, body));
        }
        if !self.matches(&Token::EndSelect) {
            return Err("Expected END SELECT".to_string());
        }
        Ok(Stmt::SelectCase {
            expr,
            cases,
            else_branch,
        })
    }
    fn parse_for(&mut self) -> Result<Stmt, String> {
        self.advance();
        let var = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected variable in FOR, got {:?}", t)),
        };
        self.declare_var_if_needed(&var);
        let vt = self.lookup_var_type(&var);
        if vt.is_string() {
            return Err(format!("FOR variable '{}' must be numeric", var));
        }
        if !self.matches(&Token::Equals) {
            return Err("Expected '=' in FOR".to_string());
        }
        let start = self.parse_expr()?;
        if !self.matches(&Token::To) {
            return Err("Expected TO in FOR".to_string());
        }
        let end = self.parse_expr()?;
        let step = if self.matches(&Token::Step) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect_numeric(&start, "FOR start")?;
        self.expect_numeric(&end, "FOR end")?;
        if let Some(s) = &step {
            self.expect_numeric(s, "FOR step")?;
        }
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Next | Token::Eof) {
                break;
            }
            body.extend(self.parse_line()?);
        }
        if !self.matches(&Token::Next) {
            return Err("Expected NEXT".to_string());
        }
        if let Token::Identifier(_) = self.peek() {
            self.advance();
        }
        Ok(Stmt::For {
            var,
            start,
            end,
            step,
            body,
        })
    }
    fn parse_while(&mut self) -> Result<Stmt, String> {
        self.advance();
        let cond = self.parse_expr()?;
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Wend | Token::Eof) {
                break;
            }
            body.extend(self.parse_line()?);
        }
        if !self.matches(&Token::Wend) {
            return Err("Expected WEND".to_string());
        }
        Ok(Stmt::While { cond, body })
    }
    fn parse_do(&mut self) -> Result<Stmt, String> {
        self.advance();
        let mut top_cond: Option<(Expr, bool, bool)> = None;
        if matches!(self.peek(), Token::While) {
            self.advance();
            top_cond = Some((self.parse_expr()?, false, true));
        } else if matches!(self.peek(), Token::Until) {
            self.advance();
            top_cond = Some((self.parse_expr()?, true, true));
        }
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Loop | Token::Eof) {
                break;
            }
            body.extend(self.parse_line()?);
        }
        if !self.matches(&Token::Loop) {
            return Err("Expected LOOP".to_string());
        }
        let cond = if top_cond.is_some() {
            top_cond
        } else if matches!(self.peek(), Token::While) {
            self.advance();
            Some((self.parse_expr()?, false, false))
        } else if matches!(self.peek(), Token::Until) {
            self.advance();
            Some((self.parse_expr()?, true, false))
        } else {
            None
        };
        Ok(Stmt::DoLoop { cond, body })
    }
    fn parse_exit(&mut self) -> Result<Stmt, String> {
        self.advance();
        let kind = match self.advance() {
            Token::For => "FOR".to_string(),
            Token::Sub => "SUB".to_string(),
            Token::Function => "FUNCTION".to_string(),
            Token::Do => "DO".to_string(),
            Token::While => "WHILE".to_string(),
            Token::Identifier(s) => s.to_uppercase(),
            t => return Err(format!("Unexpected token after EXIT: {:?}", t)),
        };
        Ok(Stmt::Exit(kind))
    }
    fn parse_dim(&mut self) -> Result<Stmt, String> {
        self.advance();
        let mut decls = Vec::new();
        loop {
            let name = match self.advance() {
                Token::Identifier(n) => n,
                t => return Err(format!("Expected name in DIM, got {:?}", t)),
            };
            let mut dims = Vec::new();
            if matches!(self.peek(), Token::LParen) {
                self.advance();
                if !matches!(self.peek(), Token::RParen) {
                    dims.push(self.parse_expr()?);
                    while matches!(self.peek(), Token::Comma) {
                        self.advance();
                        dims.push(self.parse_expr()?);
                    }
                }
                if !self.matches(&Token::RParen) {
                    return Err("Expected ')'".to_string());
                }
            }
            let vt = if matches!(self.peek(), Token::As) {
                self.advance();
                match self.advance() {
                    Token::Integer => VarType::Integer,
                    Token::Long => VarType::Long,
                    Token::Single => VarType::Single,
                    Token::Double => VarType::Double,
                    Token::StringKw => VarType::Str,
                    Token::Identifier(s) => {
                        if !self.type_defs.contains_key(&s.to_uppercase()) {
                            return Err(format!("Unknown type '{}'", s));
                        }
                        VarType::UserType(s.to_uppercase())
                    }
                    t => return Err(format!("Unexpected type in DIM: {:?}", t)),
                }
            } else {
                infer_type_from_suffix(&name)
            };
            self.var_types.insert(name.clone(), vt.clone());
            decls.push(DimDecl {
                name,
                dims,
                vtype: vt,
            });
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(Stmt::Dim(decls))
    }
    fn parse_const(&mut self) -> Result<Stmt, String> {
        self.advance();
        let name = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected name in CONST, got {:?}", t)),
        };
        if !self.matches(&Token::Equals) {
            return Err("Expected '=' in CONST".to_string());
        }
        let e = self.parse_expr()?;
        let t = infer_expr_type(
            &e,
            &self.var_types,
            &self.constants,
            &self.sub_sigs,
            &self.type_defs,
        );
        self.var_types.insert(name.clone(), t.clone());
        self.constants.insert(name.clone(), t);
        Ok(Stmt::Const(name, e))
    }
    fn parse_type_def(&mut self) -> Result<Stmt, String> {
        self.advance();
        let name = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected TYPE name, got {:?}", t)),
        };
        if !matches!(self.peek(), Token::Newline) {
            return Err("Expected newline after TYPE name".to_string());
        }
        self.advance();
        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::EndType | Token::Eof) {
                break;
            }
            let fname = match self.advance() {
                Token::Identifier(n) => n,
                t => return Err(format!("Expected field name, got {:?}", t)),
            };
            if !self.matches(&Token::As) {
                return Err("Expected AS in TYPE field".to_string());
            }
            let vt = match self.advance() {
                Token::Integer => VarType::Integer,
                Token::Long => VarType::Long,
                Token::Single => VarType::Single,
                Token::Double => VarType::Double,
                Token::StringKw => {
                    if matches!(self.peek(), Token::Star) {
                        self.advance();
                        let _ = self.parse_expr()?;
                    }
                    VarType::Str
                }
                Token::Identifier(s) => VarType::UserType(s.to_uppercase()),
                t => return Err(format!("Unexpected field type: {:?}", t)),
            };
            fields.push((fname, vt));
            if matches!(self.peek(), Token::Newline) {
                self.advance();
            }
        }
        if !self.matches(&Token::EndType) {
            return Err("Expected END TYPE".to_string());
        }
        self.type_defs.insert(name.to_uppercase(), fields.clone());
        Ok(Stmt::TypeDef(name.to_uppercase(), fields))
    }
    fn parse_sub(&mut self) -> Result<Stmt, String> {
        self.advance();
        let name = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected SUB name, got {:?}", t)),
        };
        let params = self.parse_params()?;
        let saved_types = self.var_types.clone();
        for p in &params {
            self.var_types.insert(p.name.clone(), p.vtype.clone());
        }
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::EndSub | Token::Eof) {
                break;
            }
            body.extend(self.parse_line()?);
        }
        if !self.matches(&Token::EndSub) {
            return Err("Expected END SUB".to_string());
        }
        self.var_types = saved_types;
        self.sub_sigs
            .insert(name.to_uppercase(), (params.clone(), false, VarType::Auto));
        Ok(Stmt::SubDef { name, params, body })
    }
    fn parse_function(&mut self) -> Result<Stmt, String> {
        self.advance();
        let name = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected FUNCTION name, got {:?}", t)),
        };
        let params = self.parse_params()?;
        let ret_type = if matches!(self.peek(), Token::As) {
            self.advance();
            match self.advance() {
                Token::Integer => VarType::Integer,
                Token::Long => VarType::Long,
                Token::Single => VarType::Single,
                Token::Double => VarType::Double,
                Token::StringKw => VarType::Str,
                t => return Err(format!("Unexpected return type: {:?}", t)),
            }
        } else {
            infer_type_from_suffix(&name)
        };
        let saved_types = self.var_types.clone();
        for p in &params {
            self.var_types.insert(p.name.clone(), p.vtype.clone());
        }
        self.var_types.insert(name.clone(), ret_type.clone());
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::EndFunction | Token::Eof) {
                break;
            }
            body.extend(self.parse_line()?);
        }
        if !self.matches(&Token::EndFunction) {
            return Err("Expected END FUNCTION".to_string());
        }
        self.var_types = saved_types;
        self.sub_sigs.insert(
            name.to_uppercase(),
            (params.clone(), true, ret_type.clone()),
        );
        Ok(Stmt::FuncDef {
            name,
            params,
            body,
            ret_type,
        })
    }
    fn parse_params(&mut self) -> Result<Vec<Param>, String> {
        let mut params = Vec::new();
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            if !matches!(self.peek(), Token::RParen) {
                loop {
                    let by_ref = if matches!(self.peek(), Token::ByRef) {
                        self.advance();
                        true
                    } else if matches!(self.peek(), Token::ByVal) {
                        self.advance();
                        false
                    } else {
                        true
                    };
                    let pname = match self.advance() {
                        Token::Identifier(n) => n,
                        t => return Err(format!("Expected parameter name, got {:?}", t)),
                    };
                    let mut is_array = false;
                    if matches!(self.peek(), Token::LParen) {
                        self.advance();
                        if !self.matches(&Token::RParen) {
                            return Err("Expected ')' for array param".to_string());
                        }
                        is_array = true;
                    }
                    let vt = if matches!(self.peek(), Token::As) {
                        self.advance();
                        match self.advance() {
                            Token::Integer => VarType::Integer,
                            Token::Long => VarType::Long,
                            Token::Single => VarType::Single,
                            Token::Double => VarType::Double,
                            Token::StringKw => VarType::Str,
                            Token::Identifier(s) => VarType::UserType(s.to_uppercase()),
                            t => return Err(format!("Unexpected type: {:?}", t)),
                        }
                    } else {
                        infer_type_from_suffix(&pname)
                    };
                    params.push(Param {
                        name: pname,
                        vtype: vt,
                        by_ref,
                        is_array,
                    });
                    if matches!(self.peek(), Token::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
            if !self.matches(&Token::RParen) {
                return Err("Expected ')'".to_string());
            }
        }
        Ok(params)
    }
    fn parse_call(&mut self) -> Result<Stmt, String> {
        self.advance();
        let name = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected sub name in CALL, got {:?}", t)),
        };
        let mut args = Vec::new();
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            if !matches!(self.peek(), Token::RParen) {
                args.push(self.parse_expr()?);
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
            }
            if !self.matches(&Token::RParen) {
                return Err("Expected ')'".to_string());
            }
        }
        Ok(Stmt::CallSub(name, args))
    }
    fn parse_open(&mut self) -> Result<Stmt, String> {
        self.advance();
        let filename = self.parse_expr()?;
        if !self.matches(&Token::For) {
            return Err("Expected FOR after OPEN filename".to_string());
        }
        let mode = match self.advance() {
            Token::Input => FileMode::Input,
            Token::Output => FileMode::Output,
            Token::Append => FileMode::Append,
            Token::Random => FileMode::Random,
            t => return Err(format!("Unexpected file mode: {:?}", t)),
        };
        if !self.matches(&Token::As) {
            return Err("Expected AS".to_string());
        }
        if matches!(self.peek(), Token::Hash) {
            self.advance();
        }
        let handle = match self.advance() {
            Token::Number(n) => n as u32,
            t => return Err(format!("Expected file number, got {:?}", t)),
        };
        let mut record_len = None;
        if matches!(self.peek(), Token::LenKw) {
            self.advance();
            if !self.matches(&Token::Equals) {
                return Err("Expected '=' after LEN".to_string());
            }
            record_len = Some(self.parse_expr()?);
        }
        Ok(Stmt::Open {
            filename,
            mode,
            handle,
            record_len,
        })
    }
    fn parse_close(&mut self) -> Result<Stmt, String> {
        self.advance();
        let mut handles = Vec::new();
        loop {
            if matches!(self.peek(), Token::Newline | Token::Eof | Token::Colon) {
                break;
            }
            if matches!(self.peek(), Token::Hash) {
                self.advance();
            }
            match self.advance() {
                Token::Number(n) => handles.push(n as u32),
                t => return Err(format!("Expected file number, got {:?}", t)),
            }
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(Stmt::Close(handles))
    }
    fn parse_get(&mut self) -> Result<Stmt, String> {
        self.advance();
        if matches!(self.peek(), Token::Hash) {
            self.advance();
        }
        let handle = match self.advance() {
            Token::Number(n) => n as u32,
            t => return Err(format!("Expected file number, got {:?}", t)),
        };
        let mut record = None;
        if matches!(self.peek(), Token::Comma) {
            self.advance();
            if !matches!(self.peek(), Token::Comma) {
                record = Some(self.parse_expr()?);
            }
            if !self.matches(&Token::Comma) {
                return Err("Expected ',' before variable in GET".to_string());
            }
        }
        let var = self.parse_lvalue()?;
        Ok(Stmt::Get {
            handle,
            record,
            var,
        })
    }
    fn parse_put(&mut self) -> Result<Stmt, String> {
        self.advance();
        if matches!(self.peek(), Token::Hash) {
            self.advance();
        }
        let handle = match self.advance() {
            Token::Number(n) => n as u32,
            t => return Err(format!("Expected file number, got {:?}", t)),
        };
        let mut record = None;
        if matches!(self.peek(), Token::Comma) {
            self.advance();
            if !matches!(self.peek(), Token::Comma) {
                record = Some(self.parse_expr()?);
            }
            if !self.matches(&Token::Comma) {
                return Err("Expected ',' before variable in PUT".to_string());
            }
        }
        let var = self.parse_lvalue()?;
        Ok(Stmt::Put {
            handle,
            record,
            var,
        })
    }
    fn parse_lvalue(&mut self) -> Result<LValue, String> {
        let name = match self.advance() {
            Token::Identifier(n) => n,
            t => return Err(format!("Expected variable, got {:?}", t)),
        };
        let mut lv = if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut args = Vec::new();
            if !matches!(self.peek(), Token::RParen) {
                args.push(self.parse_expr()?);
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
            }
            if !self.matches(&Token::RParen) {
                return Err("Expected ')'".to_string());
            }
            LValue::Index(name, args)
        } else {
            LValue::Var(name)
        };
        while matches!(self.peek(), Token::Dot) {
            self.advance();
            let f = match self.advance() {
                Token::Identifier(s) => s,
                t => return Err(format!("Expected field, got {:?}", t)),
            };
            lv = LValue::Field(Box::new(lv), f);
        }
        Ok(lv)
    }
    fn parse_color(&mut self) -> Result<Stmt, String> {
        self.advance();
        let mut fg = None;
        let mut bg = None;
        if !matches!(
            self.peek(),
            Token::Newline | Token::Eof | Token::Colon | Token::Comma
        ) {
            fg = Some(self.parse_expr()?);
        }
        if matches!(self.peek(), Token::Comma) {
            self.advance();
            bg = Some(self.parse_expr()?);
        }
        Ok(Stmt::Color(fg, bg))
    }
    fn parse_pset(&mut self) -> Result<Stmt, String> {
        self.advance();
        if !self.matches(&Token::LParen) {
            return Err("Expected '(' after PSET".to_string());
        }
        let x = self.parse_expr()?;
        if !self.matches(&Token::Comma) {
            return Err("Expected ','".to_string());
        }
        let y = self.parse_expr()?;
        if !self.matches(&Token::RParen) {
            return Err("Expected ')'".to_string());
        }
        let mut color = None;
        if matches!(self.peek(), Token::Comma) {
            self.advance();
            color = Some(self.parse_expr()?);
        }
        Ok(Stmt::Pset(x, y, color))
    }
    fn parse_line_stmt(&mut self) -> Result<Stmt, String> {
        self.advance();
        let mut p1 = None;
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            let x1 = self.parse_expr()?;
            if !self.matches(&Token::Comma) {
                return Err("Expected ','".to_string());
            }
            let y1 = self.parse_expr()?;
            if !self.matches(&Token::RParen) {
                return Err("Expected ')'".to_string());
            }
            p1 = Some((x1, y1));
        }
        if !self.matches(&Token::Minus) {
            return Err("Expected '-' in LINE".to_string());
        }
        if !self.matches(&Token::LParen) {
            return Err("Expected '('".to_string());
        }
        let x2 = self.parse_expr()?;
        if !self.matches(&Token::Comma) {
            return Err("Expected ','".to_string());
        }
        let y2 = self.parse_expr()?;
        if !self.matches(&Token::RParen) {
            return Err("Expected ')'".to_string());
        }
        let mut color = None;
        let mut is_box = false;
        let mut is_filled = false;
        if matches!(self.peek(), Token::Comma) {
            self.advance();
            if !matches!(self.peek(), Token::Comma) {
                color = Some(self.parse_expr()?);
            }
            if matches!(self.peek(), Token::Comma) {
                self.advance();
                if let Token::Identifier(s) = self.peek().clone() {
                    let u = s.to_uppercase();
                    if u == "B" {
                        self.advance();
                        is_box = true;
                    } else if u == "BF" {
                        self.advance();
                        is_box = true;
                        is_filled = true;
                    }
                }
            }
        }
        Ok(Stmt::Line(p1, x2, y2, color, is_box, is_filled))
    }
    fn parse_circle(&mut self) -> Result<Stmt, String> {
        self.advance();
        if !self.matches(&Token::LParen) {
            return Err("Expected '(' after CIRCLE".to_string());
        }
        let x = self.parse_expr()?;
        if !self.matches(&Token::Comma) {
            return Err("Expected ','".to_string());
        }
        let y = self.parse_expr()?;
        if !self.matches(&Token::RParen) {
            return Err("Expected ')'".to_string());
        }
        if !self.matches(&Token::Comma) {
            return Err("Expected ',' before radius".to_string());
        }
        let r = self.parse_expr()?;
        let mut color = None;
        if matches!(self.peek(), Token::Comma) {
            self.advance();
            color = Some(self.parse_expr()?);
        }
        Ok(Stmt::Circle(x, y, r, color))
    }
    fn parse_paint(&mut self) -> Result<Stmt, String> {
        self.advance();
        if !self.matches(&Token::LParen) {
            return Err("Expected '(' after PAINT".to_string());
        }
        let x = self.parse_expr()?;
        if !self.matches(&Token::Comma) {
            return Err("Expected ','".to_string());
        }
        let y = self.parse_expr()?;
        if !self.matches(&Token::RParen) {
            return Err("Expected ')'".to_string());
        }
        let mut color = None;
        let mut border = None;
        if matches!(self.peek(), Token::Comma) {
            self.advance();
            if !matches!(
                self.peek(),
                Token::Comma | Token::Newline | Token::Eof | Token::Colon
            ) {
                color = Some(self.parse_expr()?);
            }
            if matches!(self.peek(), Token::Comma) {
                self.advance();
                border = Some(self.parse_expr()?);
            }
        }
        Ok(Stmt::Paint(x, y, color, border))
    }
    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        loop {
            let op = match self.peek() {
                Token::Or => BinOp::Or,
                Token::Xor => BinOp::Xor,
                _ => break,
            };
            self.advance();
            let r = self.parse_and()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(r));
        }
        Ok(left)
    }
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Token::And) {
            self.advance();
            let r = self.parse_not()?;
            left = Expr::BinaryOp(Box::new(left), BinOp::And, Box::new(r));
        }
        Ok(left)
    }
    fn parse_not(&mut self) -> Result<Expr, String> {
        if matches!(self.peek(), Token::Not) {
            self.advance();
            let inner = self.parse_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_comparison()
    }
    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_addsub()?;
        loop {
            let op = match self.peek() {
                Token::Equals => BinOp::Eq,
                Token::NotEqual => BinOp::NotEq,
                Token::Less => BinOp::Lt,
                Token::LessEqual => BinOp::Le,
                Token::Greater => BinOp::Gt,
                Token::GreaterEqual => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let r = self.parse_addsub()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(r));
        }
        Ok(left)
    }
    fn parse_addsub(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mod()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let r = self.parse_mod()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(r));
        }
        Ok(left)
    }
    fn parse_mod(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_intdiv()?;
        while matches!(self.peek(), Token::Mod) {
            self.advance();
            let r = self.parse_intdiv()?;
            left = Expr::BinaryOp(Box::new(left), BinOp::Mod, Box::new(r));
        }
        Ok(left)
    }
    fn parse_intdiv(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_muldiv()?;
        while matches!(self.peek(), Token::Backslash) {
            self.advance();
            let r = self.parse_muldiv()?;
            left = Expr::BinaryOp(Box::new(left), BinOp::IntDiv, Box::new(r));
        }
        Ok(left)
    }
    fn parse_muldiv(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            let r = self.parse_unary()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(r));
        }
        Ok(left)
    }
    fn parse_unary(&mut self) -> Result<Expr, String> {
        if matches!(self.peek(), Token::Minus) {
            self.advance();
            let inner = self.parse_unary()?;
            return Ok(Expr::UnaryMinus(Box::new(inner)));
        }
        self.parse_pow()
    }
    fn parse_pow(&mut self) -> Result<Expr, String> {
        let base = self.parse_primary()?;
        if matches!(self.peek(), Token::Caret) {
            self.advance();
            let exp = self.parse_unary()?;
            return Ok(Expr::BinaryOp(Box::new(base), BinOp::Pow, Box::new(exp)));
        }
        Ok(base)
    }
    fn parse_primary(&mut self) -> Result<Expr, String> {
        let mut e = match self.advance() {
            Token::Number(n) => Expr::Number(n),
            Token::StringLit(s) => Expr::StringLit(s),
            Token::LenKw => {
                if !self.matches(&Token::LParen) {
                    return Err("Expected '(' after LEN".to_string());
                }
                let inner = self.parse_expr()?;
                if !self.matches(&Token::RParen) {
                    return Err("Expected ')'".to_string());
                }
                Expr::ArrayOrCall("LEN".to_string(), vec![inner])
            }
            Token::Identifier(name) => {
                if matches!(self.peek(), Token::LParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Token::RParen) {
                        args.push(self.parse_expr()?);
                        while matches!(self.peek(), Token::Comma) {
                            self.advance();
                            args.push(self.parse_expr()?);
                        }
                    }
                    if !self.matches(&Token::RParen) {
                        return Err("Expected ')'".to_string());
                    }
                    Expr::ArrayOrCall(name, args)
                } else {
                    Expr::Variable(name)
                }
            }
            Token::LParen => {
                let e = self.parse_expr()?;
                if !self.matches(&Token::RParen) {
                    return Err("Expected ')'".to_string());
                }
                e
            }
            Token::Minus => {
                let inner = self.parse_unary()?;
                Expr::UnaryMinus(Box::new(inner))
            }
            t => return Err(format!("Unexpected token in expression: {:?}", t)),
        };
        while matches!(self.peek(), Token::Dot) {
            self.advance();
            let f = match self.advance() {
                Token::Identifier(s) => s,
                t => return Err(format!("Expected field name, got {:?}", t)),
            };
            e = Expr::FieldAccess(Box::new(e), f);
        }
        Ok(e)
    }
    fn declare_var_if_needed(&mut self, name: &str) {
        if !self.var_types.contains_key(name) {
            self.var_types
                .insert(name.to_string(), infer_type_from_suffix(name));
        }
    }
    fn lookup_var_type(&self, name: &str) -> VarType {
        self.var_types
            .get(name)
            .cloned()
            .unwrap_or_else(|| infer_type_from_suffix(name))
    }
    fn lvalue_type(&self, lv: &LValue) -> VarType {
        match lv {
            LValue::Var(n) => self.lookup_var_type(n),
            LValue::Index(n, _) => self.lookup_var_type(n),
            LValue::Field(parent, fname) => {
                let pt = self.lvalue_type(parent);
                if let VarType::UserType(tn) = pt {
                    if let Some(fields) = self.type_defs.get(&tn) {
                        for (fn_, ft) in fields {
                            if fn_.eq_ignore_ascii_case(fname) {
                                return ft.clone();
                            }
                        }
                    }
                }
                infer_type_from_suffix(fname)
            }
        }
    }
    fn check_assignment_types(&self, lv: &LValue, e: &Expr) -> Result<(), String> {
        let lt = self.lvalue_type(lv);
        let et = infer_expr_type(
            e,
            &self.var_types,
            &self.constants,
            &self.sub_sigs,
            &self.type_defs,
        );
        if lt.is_string() && !et.is_string() && et != VarType::Auto {
            return Err(
                "Type mismatch: cannot assign numeric expression to string variable".to_string(),
            );
        }
        if lt.is_numeric() && et.is_string() {
            return Err(
                "Type mismatch: cannot assign string expression to numeric variable".to_string(),
            );
        }
        Ok(())
    }
    fn expect_numeric(&self, e: &Expr, ctx: &str) -> Result<(), String> {
        let t = infer_expr_type(
            e,
            &self.var_types,
            &self.constants,
            &self.sub_sigs,
            &self.type_defs,
        );
        if t.is_string() {
            return Err(format!("{} requires numeric expression", ctx));
        }
        Ok(())
    }
}

pub fn infer_type_from_suffix(name: &str) -> VarType {
    if name.ends_with('$') {
        VarType::Str
    } else if name.ends_with('%') {
        VarType::Integer
    } else if name.ends_with('&') {
        VarType::Long
    } else if name.ends_with('!') {
        VarType::Single
    } else if name.ends_with('#') {
        VarType::Double
    } else {
        VarType::Single
    }
}

pub fn infer_expr_type(
    e: &Expr,
    vars: &HashMap<String, VarType>,
    consts: &HashMap<String, VarType>,
    subs: &HashMap<String, (Vec<Param>, bool, VarType)>,
    type_defs: &HashMap<String, Vec<(String, VarType)>>,
) -> VarType {
    match e {
        Expr::Number(_) => VarType::Double,
        Expr::StringLit(_) => VarType::Str,
        Expr::Variable(n) => {
            if let Some(t) = consts.get(n) {
                return t.clone();
            }
            vars.get(n)
                .cloned()
                .unwrap_or_else(|| infer_type_from_suffix(n))
        }
        Expr::FieldAccess(parent, fname) => {
            let pt = infer_expr_type(parent, vars, consts, subs, type_defs);
            if let VarType::UserType(tn) = pt {
                if let Some(fields) = type_defs.get(&tn) {
                    for (field_name, field_type) in fields {
                        if field_name.eq_ignore_ascii_case(fname) {
                            return field_type.clone();
                        }
                    }
                }
            }
            infer_type_from_suffix(fname)
        }
        Expr::ArrayOrCall(name, _) => {
            let upper = name.to_uppercase();
            if let Some(rt) = builtin_return_type(&upper) {
                return rt;
            }
            if let Some((_, _, rt)) = subs.get(&upper) {
                return rt.clone();
            }
            vars.get(name)
                .cloned()
                .unwrap_or_else(|| infer_type_from_suffix(name))
        }
        Expr::UnaryMinus(_) | Expr::Not(_) => VarType::Double,
        Expr::BinaryOp(l, op, r) => {
            let lt = infer_expr_type(l, vars, consts, subs, type_defs);
            let rt = infer_expr_type(r, vars, consts, subs, type_defs);
            match op {
                BinOp::Add => {
                    if lt.is_string() || rt.is_string() {
                        VarType::Str
                    } else {
                        VarType::Double
                    }
                }
                BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    VarType::Integer
                }
                _ => VarType::Double,
            }
        }
    }
}

fn builtin_return_type(name: &str) -> Option<VarType> {
    let t = match name {
        "LEN" | "INT" | "ABS" | "SQR" | "RND" | "VAL" | "ASC" | "INSTR" | "SIN" | "COS" | "TAN"
        | "ATN" | "LOG" | "EXP" | "SGN" | "TIMER" => VarType::Double,
        "MID$" | "MID" | "LEFT$" | "LEFT" | "RIGHT$" | "RIGHT" | "STR$" | "CHR$" | "UCASE$"
        | "LCASE$" | "SPACE$" | "STRING$" | "HEX$" | "OCT$" => VarType::Str,
        _ => return None,
    };
    Some(t)
}
