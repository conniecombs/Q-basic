#![allow(dead_code)]
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::graphics::{qb_color, SharedGraphics};
use crate::parser::{
    infer_type_from_suffix, BinOp, CaseClause, DataItem, DimDecl, Expr, FileMode, LValue, Param,
    PrintItem, Program, Stmt, VarType,
};

#[derive(Debug, Clone)]
pub enum Value {
    Number(f64),
    Str(String),
    Array(ArrayValue),
    Record(RecordValue),
}

#[derive(Debug, Clone)]
pub struct ArrayValue {
    pub dims: Vec<usize>,
    pub data: Vec<Value>,
    pub element_type: VarType,
}

#[derive(Debug, Clone)]
pub struct RecordValue {
    pub type_name: String,
    pub fields: Vec<(String, Value)>,
}

impl Value {
    pub fn to_display(&self) -> String {
        match self {
            Value::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e16 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            Value::Str(s) => s.clone(),
            Value::Array(_) => "[array]".to_string(),
            Value::Record(_) => "[record]".to_string(),
        }
    }
    pub fn as_number(&self) -> Result<f64, String> {
        match self {
            Value::Number(n) => Ok(*n),
            _ => Err("Type mismatch: expected number".to_string()),
        }
    }
    pub fn as_string(&self) -> Result<String, String> {
        match self {
            Value::Str(s) => Ok(s.clone()),
            Value::Number(n) => Ok(format!("{}", n)),
            _ => Err("Type mismatch: expected string".to_string()),
        }
    }
}

pub enum ConsoleMsg {
    Print { text: String, newline: bool },
    InputPrompt(String),
    Clear,
    End(Option<String>),
}

enum Flow {
    Normal,
    Goto(String),
    Gosub(String, usize),
    Return,
    End,
    ExitFor,
    ExitDoWhile,
    ExitSub,
}

pub enum FileHandle {
    Input(BufReader<File>),
    Output(BufWriter<File>),
    Append(BufWriter<File>),
    Random { file: File, record_len: usize },
}

pub struct SubInfo {
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub is_function: bool,
    pub ret_type: VarType,
}

impl SubInfo {
    fn clone_meta(&self) -> SubInfo {
        SubInfo {
            params: self.params.clone(),
            body: self.body.clone(),
            is_function: self.is_function,
            ret_type: self.ret_type.clone(),
        }
    }
}

pub struct Scope {
    pub vars: HashMap<String, Value>,
}
impl Scope {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
        }
    }
}

pub struct Interpreter {
    pub labels: HashMap<String, usize>,
    pub subs: HashMap<String, SubInfo>,
    pub types: HashMap<String, Vec<(String, VarType)>>,
    pub constants: HashMap<String, Value>,
    pub data_values: Vec<Value>,
    pub data_ptr: usize,
    pub data_labels: HashMap<String, usize>,
    pub scopes: Vec<Scope>,
    pub files: HashMap<u32, FileHandle>,
    pub default_types: Vec<(char, char, VarType)>,
    pub gosub_stack: Vec<usize>,
    pub rng_state: u64,
    pub console_tx: Sender<ConsoleMsg>,
    pub input_rx: Receiver<String>,
    pub cancel_flag: Arc<AtomicBool>,
    pub graphics: Arc<Mutex<SharedGraphics>>,
}

pub fn run(
    program: Program,
    console_tx: Sender<ConsoleMsg>,
    input_rx: Receiver<String>,
    cancel_flag: Arc<AtomicBool>,
    graphics: Arc<Mutex<SharedGraphics>>,
) -> Result<(), String> {
    let stmts = program.stmts;
    let mut interp = Interpreter {
        labels: HashMap::new(),
        subs: HashMap::new(),
        types: HashMap::new(),
        constants: HashMap::new(),
        data_values: Vec::new(),
        data_ptr: 0,
        data_labels: HashMap::new(),
        scopes: vec![Scope::new()],
        files: HashMap::new(),
        default_types: Vec::new(),
        gosub_stack: Vec::new(),
        rng_state: 0x12345678,
        console_tx,
        input_rx,
        cancel_flag,
        graphics,
    };

    let mut exec_stmts = Vec::new();
    for s in stmts {
        match s {
            Stmt::SubDef { name, params, body } => {
                interp.subs.insert(
                    name.clone(),
                    SubInfo {
                        params,
                        body,
                        is_function: false,
                        ret_type: VarType::Auto,
                    },
                );
            }
            Stmt::FuncDef {
                name,
                params,
                body,
                ret_type,
            } => {
                interp.subs.insert(
                    name.clone(),
                    SubInfo {
                        params,
                        body,
                        is_function: true,
                        ret_type,
                    },
                );
            }
            Stmt::TypeDef(name, fields) => {
                interp.types.insert(name, fields);
            }
            other => exec_stmts.push(other),
        }
    }
    for (i, s) in exec_stmts.iter().enumerate() {
        match s {
            Stmt::Label(n) => {
                interp.labels.insert(n.clone(), i);
                interp
                    .data_labels
                    .insert(n.clone(), interp.data_values.len());
            }
            Stmt::LineNumber(n) => {
                interp.labels.insert(format!("{}", n), i);
                interp
                    .data_labels
                    .insert(format!("{}", n), interp.data_values.len());
            }
            Stmt::Data(items) => {
                interp
                    .data_values
                    .extend(items.iter().map(data_item_to_value));
            }
            _ => {}
        }
    }
    let const_stmts: Vec<_> = exec_stmts
        .iter()
        .filter_map(|s| {
            if let Stmt::Const(n, e) = s {
                Some((n.clone(), e.clone()))
            } else {
                None
            }
        })
        .collect();
    for (n, e) in const_stmts {
        let v = interp.eval(&e)?;
        interp.constants.insert(n, v);
    }

    let mut pc = 0;
    while pc < exec_stmts.len() {
        let stmt = &exec_stmts[pc]; // OPTIMIZATION: Process AST by reference
        let next_pc = pc + 1;
        match interp.exec_stmt(stmt)? {
            Flow::Normal => pc = next_pc,
            Flow::Goto(t) => {
                pc = *interp
                    .labels
                    .get(&t)
                    .ok_or_else(|| format!("Undefined label: {}", t))?;
            }
            Flow::Gosub(t, _) => {
                interp.gosub_stack.push(next_pc);
                pc = *interp
                    .labels
                    .get(&t)
                    .ok_or_else(|| format!("Undefined label: {}", t))?;
            }
            Flow::Return => {
                pc = interp
                    .gosub_stack
                    .pop()
                    .ok_or_else(|| "RETURN without GOSUB".to_string())?;
            }
            Flow::End => break,
            _ => pc = next_pc,
        }
    }
    Ok(())
}

impl Interpreter {
    fn current_scope(&self) -> &Scope {
        self.scopes.last().unwrap()
    }
    fn current_scope_mut(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }
    fn infer_runtime_type(&self, name: &str) -> VarType {
        let suffixed = infer_type_from_suffix(name);
        if !matches!(suffixed, VarType::Single) || name.ends_with('!') {
            return suffixed;
        }
        let Some(first) = name.chars().find(|c| c.is_ascii_alphabetic()) else {
            return suffixed;
        };
        let first = first.to_ascii_uppercase();
        self.default_types
            .iter()
            .rev()
            .find(|(start, end, _)| first >= *start && first <= *end)
            .map(|(_, _, vt)| vt.clone())
            .unwrap_or(suffixed)
    }
    fn get_var(&self, name: &str) -> Value {
        if let Some(v) = self.constants.get(name) {
            return v.clone();
        }
        if let Some(v) = self.current_scope().vars.get(name) {
            return v.clone();
        }
        if self.scopes.len() > 1 {
            if let Some(v) = self.scopes[0].vars.get(name) {
                return v.clone();
            }
        }
        let t = self.infer_runtime_type(name);
        if t.is_string() {
            Value::Str(String::new())
        } else {
            Value::Number(0.0)
        }
    }
    fn get_var_ref(&self, name: &str) -> Option<&Value> {
        self.current_scope().vars.get(name).or_else(|| {
            if self.scopes.len() > 1 {
                self.scopes[0].vars.get(name)
            } else {
                None
            }
        })
    }
    fn set_var(&mut self, name: &str, v: Value) -> Result<(), String> {
        if self.constants.contains_key(name) {
            return Err(format!("Cannot assign to constant '{}'", name));
        }
        self.current_scope_mut().vars.insert(name.to_string(), v);
        Ok(())
    }
    fn default_value_for_type(&self, vt: &VarType) -> Value {
        match vt {
            VarType::Str => Value::Str(String::new()),
            VarType::UserType(tn) => {
                if let Some(fields) = self.types.get(tn) {
                    let fs: Vec<(String, Value)> = fields
                        .iter()
                        .map(|(n, t)| (n.clone(), self.default_value_for_type(t)))
                        .collect();
                    Value::Record(RecordValue {
                        type_name: tn.clone(),
                        fields: fs,
                    })
                } else {
                    Value::Number(0.0)
                }
            }
            _ => Value::Number(0.0),
        }
    }
    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<Flow, String> {
        if self.cancel_flag.load(Ordering::Relaxed) {
            return Err("Execution interrupted by user".to_string());
        }

        match stmt {
            Stmt::LineNumber(_)
            | Stmt::Label(_)
            | Stmt::Const(_, _)
            | Stmt::Noop
            | Stmt::TypeDef(_, _)
            | Stmt::Data(_) => Ok(Flow::Normal),
            Stmt::DefType(vtype, ranges) => {
                for (start, end) in ranges {
                    self.default_types.push((*start, *end, vtype.clone()));
                }
                Ok(Flow::Normal)
            }
            Stmt::Print(handle, items) => self.do_print(*handle, items),
            Stmt::Let(lv, expr) => {
                let v = self.eval(expr)?;
                self.assign(lv, v)?;
                Ok(Flow::Normal)
            }
            Stmt::Input(handle, prompt, vars) => self.do_input(*handle, prompt.as_deref(), vars),
            Stmt::If {
                cond,
                then_branch,
                elseifs,
                else_branch,
            } => {
                let v = self.eval(cond)?;
                if is_truthy(&v) {
                    return self.exec_block(then_branch);
                }
                for (c, b) in elseifs {
                    let cv = self.eval(c)?;
                    if is_truthy(&cv) {
                        return self.exec_block(b);
                    }
                }
                self.exec_block(else_branch)
            }
            Stmt::SelectCase {
                expr,
                cases,
                else_branch,
            } => {
                let target = self.eval(expr)?;
                for (clauses, body) in cases {
                    let mut matched = false;
                    for cl in clauses {
                        if self.case_matches(&target, cl)? {
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        return self.exec_block(body);
                    }
                }
                self.exec_block(else_branch)
            }
            Stmt::For {
                var,
                start,
                end,
                step,
                body,
            } => {
                let s = self.eval(start)?.as_number()?;
                let e = self.eval(end)?.as_number()?;
                let st = match step {
                    Some(x) => self.eval(x)?.as_number()?,
                    None => 1.0,
                };
                if st == 0.0 {
                    return Err("FOR step zero".to_string());
                }
                self.set_var(var, Value::Number(s))?;
                loop {
                    let cur = self.get_var(var).as_number()?;
                    let cont = if st > 0.0 { cur <= e } else { cur >= e };
                    if !cont {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Normal => {}
                        Flow::ExitFor => break,
                        other => return Ok(other),
                    }
                    let cur = self.get_var(var).as_number()?;
                    self.set_var(var, Value::Number(cur + st))?;
                }
                Ok(Flow::Normal)
            }
            Stmt::While { cond, body } => {
                loop {
                    let v = self.eval(cond)?;
                    if !is_truthy(&v) {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Normal => {}
                        Flow::ExitDoWhile => break,
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::DoLoop { cond, body } => {
                loop {
                    if let Some((c, is_until, true)) = cond {
                        let v = self.eval(c)?;
                        let t = is_truthy(&v);
                        if (*is_until && t) || (!*is_until && !t) {
                            break;
                        }
                    }
                    match self.exec_block(body)? {
                        Flow::Normal => {}
                        Flow::ExitDoWhile => break,
                        other => return Ok(other),
                    }
                    if let Some((c, is_until, false)) = cond {
                        let v = self.eval(c)?;
                        let t = is_truthy(&v);
                        if (*is_until && t) || (!*is_until && !t) {
                            break;
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Goto(t) => Ok(Flow::Goto(t.clone())),
            Stmt::Gosub(t) => Ok(Flow::Gosub(t.clone(), 0)),
            Stmt::Return => Ok(Flow::Return),
            Stmt::End => Ok(Flow::End),
            Stmt::Exit(kind) => match kind.as_str() {
                "FOR" => Ok(Flow::ExitFor),
                "DO" | "WHILE" => Ok(Flow::ExitDoWhile),
                "SUB" | "FUNCTION" => Ok(Flow::ExitSub),
                _ => Err(format!("Unknown EXIT type: {}", kind)),
            },
            Stmt::Dim(decls) => {
                for d in decls {
                    self.do_dim(d)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::Redim(decls) => {
                for d in decls {
                    self.do_dim(d)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::Erase(names) => {
                for name in names {
                    self.current_scope_mut().vars.remove(name);
                    if self.scopes.len() > 1 {
                        self.scopes[0].vars.remove(name);
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Read(vars) => self.do_read(vars),
            Stmt::Restore(target) => {
                self.do_restore(target.as_deref())?;
                Ok(Flow::Normal)
            }
            Stmt::SubDef { .. } | Stmt::FuncDef { .. } => Ok(Flow::Normal),
            Stmt::CallSub(name, args) => {
                self.invoke_sub(name, args)?;
                Ok(Flow::Normal)
            }
            Stmt::Open {
                filename,
                mode,
                handle,
                record_len,
            } => {
                let fname = self.eval(filename)?.as_string()?;
                let rlen = match record_len {
                    Some(e) => Some(self.eval(e)?.as_number()? as usize),
                    None => {
                        if matches!(mode, FileMode::Random) {
                            Some(128)
                        } else {
                            None
                        }
                    }
                };
                let entry = match mode {
                    FileMode::Input => {
                        let f = File::open(&fname).map_err(|e| format!("OPEN error: {}", e))?;
                        FileHandle::Input(BufReader::new(f))
                    }
                    FileMode::Output => {
                        let f = File::create(&fname).map_err(|e| format!("OPEN error: {}", e))?;
                        FileHandle::Output(BufWriter::new(f))
                    }
                    FileMode::Append => {
                        let f = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&fname)
                            .map_err(|e| format!("OPEN error: {}", e))?;
                        FileHandle::Append(BufWriter::new(f))
                    }
                    FileMode::Random => {
                        let f = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(&fname)
                            .map_err(|e| format!("OPEN error: {}", e))?;
                        FileHandle::Random { file: f, record_len: rlen.unwrap_or(128) }
                    }
                };
                self.files.insert(*handle, entry);
                Ok(Flow::Normal)
            }
            Stmt::Close(handles) => {
                if handles.is_empty() {
                    self.files.clear();
                } else {
                    for h in handles {
                        self.files.remove(h);
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Get {
                handle,
                record,
                var,
            } => self.do_get(*handle, record.as_ref(), var),
            Stmt::Put {
                handle,
                record,
                var,
            } => self.do_put(*handle, record.as_ref(), var),
            Stmt::Screen(e) => {
                let mode = self.eval(e)?.as_number()? as i32;
                let mut g = self.graphics.lock().unwrap();
                g.screen(mode);
                Ok(Flow::Normal)
            }
            Stmt::Window(_) => Ok(Flow::Normal), // Stub: Multiple windows not supported in shared buffer
            Stmt::Cls => {
                let mut g = self.graphics.lock().unwrap();
                g.cls();
                let _ = self.console_tx.send(ConsoleMsg::Clear);
                Ok(Flow::Normal)
            }
            Stmt::Color(fg, bg) => {
                let fv = match fg {
                    Some(e) => Some(qb_color(self.eval(e)?.as_number()? as u32)),
                    None => None,
                };
                let bv = match bg {
                    Some(e) => Some(qb_color(self.eval(e)?.as_number()? as u32)),
                    None => None,
                };
                let mut g = self.graphics.lock().unwrap();
                if let Some(c) = fv {
                    g.fg_color = c;
                }
                if let Some(c) = bv {
                    g.bg_color = c;
                }
                Ok(Flow::Normal)
            }
            Stmt::Pset(x, y, color) => {
                let xv = self.eval(x)?.as_number()? as i32;
                let yv = self.eval(y)?.as_number()? as i32;
                let cv = match color {
                    Some(c) => Some(qb_color(self.eval(c)?.as_number()? as u32)),
                    None => None,
                };
                let mut g = self.graphics.lock().unwrap();
                g.pset(xv, yv, cv);
                Ok(Flow::Normal)
            }
            Stmt::Line(p1, x2, y2, color, is_box, is_filled) => {
                let (x1v, y1v) = if let Some((a, b)) = p1 {
                    (
                        self.eval(a)?.as_number()? as i32,
                        self.eval(b)?.as_number()? as i32,
                    )
                } else {
                    (0, 0)
                };
                let x2v = self.eval(x2)?.as_number()? as i32;
                let y2v = self.eval(y2)?.as_number()? as i32;
                let cv = match color {
                    Some(c) => Some(qb_color(self.eval(c)?.as_number()? as u32)),
                    None => None,
                };
                let mut g = self.graphics.lock().unwrap();
                if *is_box {
                    g.rect(x1v, y1v, x2v, y2v, cv, *is_filled);
                } else {
                    g.line(x1v, y1v, x2v, y2v, cv);
                }
                Ok(Flow::Normal)
            }
            Stmt::Circle(x, y, r, color) => {
                let xv = self.eval(x)?.as_number()? as i32;
                let yv = self.eval(y)?.as_number()? as i32;
                let rv = self.eval(r)?.as_number()? as i32;
                let cv = match color {
                    Some(c) => Some(qb_color(self.eval(c)?.as_number()? as u32)),
                    None => None,
                };
                let mut g = self.graphics.lock().unwrap();
                g.circle(xv, yv, rv, cv);
                Ok(Flow::Normal)
            }
            Stmt::Paint(x, y, fill, border) => {
                let xv = self.eval(x)?.as_number()? as i32;
                let yv = self.eval(y)?.as_number()? as i32;
                let fv = match fill {
                    Some(c) => qb_color(self.eval(c)?.as_number()? as u32),
                    None => 0xFFFFFF,
                };
                let bv = match border {
                    Some(c) => qb_color(self.eval(c)?.as_number()? as u32),
                    None => fv,
                };
                let mut g = self.graphics.lock().unwrap();
                g.paint(xv, yv, fv, bv);
                Ok(Flow::Normal)
            }
            Stmt::Locate(row, col) => {
                let r = match row {
                    Some(e) => self.eval(e)?.as_number()? as u32,
                    None => 1,
                };
                let c = match col {
                    Some(e) => self.eval(e)?.as_number()? as u32,
                    None => 1,
                };
                let _ = self.console_tx.send(ConsoleMsg::Print {
                    text: format!("\x1B[{};{}H", r.max(1), c.max(1)),
                    newline: false,
                });
                Ok(Flow::Normal)
            }
            Stmt::Beep => {
                let _ = self.console_tx.send(ConsoleMsg::Print {
                    text: "\x07".to_string(),
                    newline: false,
                });
                Ok(Flow::Normal)
            }
            Stmt::Swap(left, right) => {
                let lv = self.read_lvalue(left)?;
                let rv = self.read_lvalue(right)?;
                self.assign(left, rv)?;
                self.assign(right, lv)?;
                Ok(Flow::Normal)
            }
            Stmt::Clear => {
                self.current_scope_mut().vars.clear();
                Ok(Flow::Normal)
            }
            Stmt::Stop | Stmt::System => Ok(Flow::End),
            Stmt::OnJump {
                selector,
                targets,
                is_gosub,
            } => {
                let idx = self.eval(selector)?.as_number()? as isize;
                if idx < 1 || idx as usize > targets.len() {
                    Ok(Flow::Normal)
                } else {
                    let target = targets[idx as usize - 1].clone();
                    if *is_gosub {
                        Ok(Flow::Gosub(target, 0))
                    } else {
                        Ok(Flow::Goto(target))
                    }
                }
            }
            Stmt::Randomize(seed) => {
                self.rng_state = match seed {
                    Some(expr) => seed_from_number(self.eval(expr)?.as_number()?),
                    None => seed_from_time(),
                };
                Ok(Flow::Normal)
            }
            Stmt::Sleep(arg) => {
                let secs = match arg {
                    Some(e) => self.eval(e)?.as_number()? as u64,
                    None => 1,
                };
                let chunks = secs * 10;
                for _ in 0..chunks {
                    if self.cancel_flag.load(Ordering::Relaxed) {
                        return Err("Execution interrupted by user".to_string());
                    }
                    let _ = self.input_rx.recv_timeout(std::time::Duration::from_millis(100));
                }
                Ok(Flow::Normal)
            }
        }
    }
    fn case_matches(&mut self, target: &Value, cl: &CaseClause) -> Result<bool, String> {
        match cl {
            CaseClause::Value(e) => {
                let v = self.eval(e)?;
                let r = apply_binop(target, &BinOp::Eq, &v)?;
                Ok(is_truthy(&r))
            }
            CaseClause::Range(a, b) => {
                let av = self.eval(a)?;
                let bv = self.eval(b)?;
                let ge = apply_binop(target, &BinOp::Ge, &av)?;
                let le = apply_binop(target, &BinOp::Le, &bv)?;
                Ok(is_truthy(&ge) && is_truthy(&le))
            }
            CaseClause::Compare(op, e) => {
                let v = self.eval(e)?;
                let r = apply_binop(target, op, &v)?;
                Ok(is_truthy(&r))
            }
        }
    }
    fn do_dim(&mut self, d: &DimDecl) -> Result<(), String> {
        if d.dims.is_empty() {
            let init = self.default_value_for_type(&d.vtype);
            self.set_var(&d.name, init)?;
        } else {
            let mut sizes = Vec::new();
            for de in &d.dims {
                let n = self.eval(de)?.as_number()? as i64;
                if n < 0 {
                    return Err("Negative array size".to_string());
                }
                sizes.push((n + 1) as usize);
            }
            let total: usize = sizes.iter().product();
            let init = self.default_value_for_type(&d.vtype);
            let arr = ArrayValue {
                dims: sizes,
                data: vec![init; total],
                element_type: d.vtype.clone(),
            };
            self.set_var(&d.name, Value::Array(arr))?;
        }
        Ok(())
    }
    fn do_read(&mut self, vars: &[LValue]) -> Result<Flow, String> {
        for lv in vars {
            let value = self
                .data_values
                .get(self.data_ptr)
                .cloned()
                .ok_or_else(|| "READ past end of DATA".to_string())?;
            self.data_ptr += 1;
            self.assign(lv, value)?;
        }
        Ok(Flow::Normal)
    }
    fn do_restore(&mut self, target: Option<&str>) -> Result<(), String> {
        self.data_ptr = match target {
            Some(label) => *self
                .data_labels
                .get(label)
                .ok_or_else(|| format!("RESTORE target '{}' does not exist", label))?,
            None => 0,
        };
        Ok(())
    }
    fn exec_block(&mut self, body: &[Stmt]) -> Result<Flow, String> {
        let mut i = 0;
        while i < body.len() {
            let s = &body[i]; // OPTIMIZATION: Reference iteration
            match self.exec_stmt(s)? {
                Flow::Normal => i += 1,
                Flow::Goto(t) => {
                    if let Some(idx) = block_label(body, &t) {
                        i = idx;
                    } else {
                        return Ok(Flow::Goto(t));
                    }
                }
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }
    fn do_print(&mut self, handle: Option<u32>, items: &[PrintItem]) -> Result<Flow, String> {
        let mut out = String::new();
        let mut newline = true;
        for it in items {
            match it {
                PrintItem::Expr(e) => {
                    let v = self.eval(e)?;
                    out.push_str(&v.to_display());
                    newline = true;
                }
                PrintItem::Semicolon => {
                    newline = false;
                }
                PrintItem::Comma => {
                    out.push('\t');
                    newline = false;
                }
            }
        }
        if let Some(h) = handle {
            let handle_enum = self
                .files
                .get_mut(&h)
                .ok_or_else(|| format!("File #{} not open", h))?;
            let w = match handle_enum {
                FileHandle::Output(w) | FileHandle::Append(w) => w,
                _ => return Err("File not open for output".to_string()),
            };
            if newline {
                writeln!(w, "{}", out).map_err(|e| e.to_string())?;
            } else {
                write!(w, "{}", out).map_err(|e| e.to_string())?;
            }
            w.flush().ok();
        } else {
            let _ = self
                .console_tx
                .send(ConsoleMsg::Print { text: out, newline });
        }
        Ok(Flow::Normal)
    }
    fn do_input(
        &mut self,
        handle: Option<u32>,
        prompt: Option<&str>,
        vars: &[LValue],
    ) -> Result<Flow, String> {
        let line = if let Some(h) = handle {
            let handle_enum = self
                .files
                .get_mut(&h)
                .ok_or_else(|| format!("File #{} not open", h))?;
            let r = match handle_enum {
                FileHandle::Input(r) => r,
                _ => return Err("File not open for input".to_string()),
            };
            let mut buf = String::new();
            r.read_line(&mut buf).map_err(|e| e.to_string())?;
            buf.trim_end_matches(&['\r', '\n'][..]).to_string()
        } else {
            let prompt_str = prompt.unwrap_or("? ");
            let _ = self
                .console_tx
                .send(ConsoleMsg::InputPrompt(prompt_str.to_string()));
            self.input_rx.recv().unwrap_or_else(|_| String::new())
        };

        let parts: Vec<&str> = if vars.len() > 1 {
            line.split(',').collect()
        } else {
            vec![&line]
        };
        for (i, lv) in vars.iter().enumerate() {
            let raw = parts.get(i).map(|s| s.trim()).unwrap_or("");
            let target_type = self.lvalue_runtime_type(lv);
            let v = if target_type.is_string() {
                Value::Str(raw.to_string())
            } else {
                let n: f64 = raw
                    .parse()
                    .map_err(|_| format!("INPUT expected a number, got '{}'", raw))?;
                Value::Number(n)
            };
            self.assign(lv, v)?;
        }
        Ok(Flow::Normal)
    }
    fn lvalue_runtime_type(&self, lv: &LValue) -> VarType {
        match lv {
            LValue::Var(n) => self
                .get_var_ref(n)
                .map(|v| runtime_type_for_value(n, v))
                .unwrap_or_else(|| self.infer_runtime_type(n)),
            LValue::Index(n, _) => {
                if let Some(Value::Array(a)) = self
                    .current_scope()
                    .vars
                    .get(n)
                    .or_else(|| self.scopes[0].vars.get(n))
                {
                    a.element_type.clone()
                } else {
                    self.infer_runtime_type(n)
                }
            }
            LValue::Field(parent, fname) => self
                .peek_lvalue_value(parent)
                .and_then(|v| match v {
                    Value::Record(rec) => rec
                        .fields
                        .iter()
                        .find(|(field_name, _)| field_name.eq_ignore_ascii_case(fname))
                        .map(|(field_name, value)| runtime_type_for_value(field_name, value)),
                    _ => None,
                })
                .unwrap_or_else(|| self.infer_runtime_type(fname)),
        }
    }
    fn peek_lvalue_value<'a>(&'a self, lv: &LValue) -> Option<&'a Value> {
        match lv {
            LValue::Var(n) => self.get_var_ref(n),
            LValue::Field(parent, fname) => match self.peek_lvalue_value(parent)? {
                Value::Record(rec) => rec
                    .fields
                    .iter()
                    .find(|(field_name, _)| field_name.eq_ignore_ascii_case(fname))
                    .map(|(_, value)| value),
                _ => None,
            },
            LValue::Index(_, _) => None,
        }
    }
    fn assign(&mut self, lv: &LValue, v: Value) -> Result<(), String> {
        match lv {
            LValue::Var(n) => {
                if self.constants.contains_key(n) {
                    return Err(format!("Cannot assign to constant '{}'", n));
                }
                let existing = self.get_var_ref(n).cloned();
                let inferred = self.infer_runtime_type(n);
                let coerced = coerce_to_existing_shape(n, existing.as_ref(), v, &inferred)?;
                self.set_var(n, coerced)?;
                Ok(())
            }
            LValue::Index(name, idx_exprs) => {
                let mut idxs = Vec::new();
                for e in idx_exprs {
                    idxs.push(self.eval(e)?.as_number()? as usize);
                }
                let scope_idx = if self.current_scope().vars.contains_key(&name.clone()) {
                    self.scopes.len() - 1
                } else if self.scopes[0].vars.contains_key(&name.clone()) {
                    0
                } else {
                    return Err(format!("Undefined array '{}'", name));
                };
                if let Some(Value::Array(a)) = self.scopes[scope_idx].vars.get_mut(name) {
                    let flat = flatten_index(&a.dims, &idxs)?;
                    let coerced = if a.element_type.is_string() {
                        Value::Str(v.as_string()?)
                    } else {
                        Value::Number(v.as_number()?)
                    };
                    a.data[flat] = coerced;
                    Ok(())
                } else {
                    Err(format!("'{}' is not an array", name))
                }
            }
            LValue::Field(parent, fname) => self.assign_field(parent, fname, v),
        }
    }
    fn assign_field(&mut self, parent: &LValue, fname: &str, v: Value) -> Result<(), String> {
        match parent {
            LValue::Var(n) => {
                let scope_idx = if self.current_scope().vars.contains_key(n) {
                    self.scopes.len() - 1
                } else if self.scopes[0].vars.contains_key(n) {
                    0
                } else {
                    return Err(format!("Undefined variable '{}'", n));
                };
                if let Some(Value::Record(rec)) = self.scopes[scope_idx].vars.get_mut(n) {
                    for (fn_, fv) in rec.fields.iter_mut() {
                        if fn_.eq_ignore_ascii_case(fname) {
                            let existing = fv.clone();
                            let inferred = infer_type_from_suffix(fn_);
                            *fv = coerce_to_existing_shape(fn_, Some(&existing), v, &inferred)?;
                            return Ok(());
                        }
                    }
                    return Err(format!("Field '{}' not found", fname));
                }
                Err(format!("'{}' is not a record", n))
            }
            _ => Err("Nested field assignment not yet supported".to_string()),
        }
    }
    fn do_get(&mut self, handle: u32, record: Option<&Expr>, var: &LValue) -> Result<Flow, String> {
        let rec_num = match record {
            Some(e) => Some(self.eval(e)?.as_number()? as u64),
            None => None,
        };
        let (rlen, buf) = {
            let handle_enum = self
                .files
                .get_mut(&handle)
                .ok_or_else(|| format!("File #{} not open", handle))?;
            let (f, rlen) = match handle_enum {
                FileHandle::Random { file, record_len } => (file, *record_len),
                _ => return Err("GET requires RANDOM mode".to_string()),
            };
            if let Some(rn) = rec_num {
                let pos = (rn.saturating_sub(1)) * rlen as u64;
                f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
            }
            let mut buf = vec![0u8; rlen];
            f.read_exact(&mut buf)
                .map_err(|e| format!("GET read error: {}", e))?;
            (rlen, buf)
        };
        let val = self.deserialize(&self.lvalue_runtime_type(var), &mut buf.as_slice(), rlen)?;
        self.assign(var, val)?;
        Ok(Flow::Normal)
    }
    fn do_put(&mut self, handle: u32, record: Option<&Expr>, var: &LValue) -> Result<Flow, String> {
        let rec_num = match record {
            Some(e) => Some(self.eval(e)?.as_number()? as u64),
            None => None,
        };
        let value = self.read_lvalue(var)?;
        let handle_enum = self
            .files
            .get_mut(&handle)
            .ok_or_else(|| format!("File #{} not open", handle))?;
        let (f, rlen) = match handle_enum {
            FileHandle::Random { file, record_len } => (file, *record_len),
            _ => return Err("PUT requires RANDOM mode".to_string()),
        };
        if let Some(rn) = rec_num {
            let pos = (rn.saturating_sub(1)) * rlen as u64;
            f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        }
        let mut buf = vec![0u8; rlen];
        serialize(&value, &mut buf);
        f.write_all(&buf)
            .map_err(|e| format!("PUT write error: {}", e))?;
        f.flush().ok();
        Ok(Flow::Normal)
    }
    fn deserialize(&self, vt: &VarType, buf: &mut &[u8], rlen: usize) -> Result<Value, String> {
        match vt {
            VarType::Str => {
                let s: String = buf
                    .iter()
                    .take(rlen)
                    .map(|b| *b as char)
                    .collect::<String>()
                    .trim_end_matches('\0')
                    .to_string();
                Ok(Value::Str(s))
            }
            VarType::UserType(tn) => {
                let fields = self
                    .types
                    .get(tn)
                    .ok_or_else(|| format!("Unknown type '{}'", tn))?
                    .clone();
                let mut record_fields = Vec::new();
                let mut offset = 0;
                for (fn_, ft) in fields {
                    let fsize = type_size(&ft);
                    if offset + fsize > buf.len() {
                        break;
                    }
                    let mut slice = &buf[offset..offset + fsize];
                    let v = self.deserialize(&ft, &mut slice, fsize)?;
                    record_fields.push((fn_, v));
                    offset += fsize;
                }
                Ok(Value::Record(RecordValue {
                    type_name: tn.clone(),
                    fields: record_fields,
                }))
            }
            _ => {
                if buf.len() < 8 {
                    return Ok(Value::Number(0.0));
                }
                let arr: [u8; 8] = buf[..8].try_into().unwrap();
                Ok(Value::Number(f64::from_le_bytes(arr)))
            }
        }
    }
    fn read_lvalue(&mut self, lv: &LValue) -> Result<Value, String> {
        match lv {
            LValue::Var(n) => Ok(self.get_var(n)),
            LValue::Index(n, idx_exprs) => {
                let mut idxs = Vec::new();
                for e in idx_exprs {
                    idxs.push(self.eval(e)?.as_number()? as usize);
                }
                let v = self.get_var_ref(n);
                if let Some(Value::Array(a)) = v {
                    let flat = flatten_index(&a.dims, &idxs)?;
                    Ok(a.data[flat].clone())
                } else {
                    Err(format!("'{}' is not an array", n))
                }
            }
            LValue::Field(parent, fname) => match &**parent {
                LValue::Var(n) => {
                    let pv = self.get_var_ref(n).ok_or_else(|| format!("Undefined variable '{}'", n))?;
                    if let Value::Record(rec) = pv {
                        for (fn_, fv) in &rec.fields {
                            if fn_.eq_ignore_ascii_case(fname) {
                                return Ok(fv.clone());
                            }
                        }
                        Err(format!("Field '{}' not found", fname))
                    } else {
                        Err("Not a record".to_string())
                    }
                }
                _ => {
                    let pv = self.read_lvalue(parent)?;
                    if let Value::Record(rec) = pv {
                        for (fn_, fv) in rec.fields {
                            if fn_.eq_ignore_ascii_case(fname) {
                                return Ok(fv);
                            }
                        }
                        Err(format!("Field '{}' not found", fname))
                    } else {
                        Err("Not a record".to_string())
                    }
                }
            },
        }
    }
    fn eval(&mut self, e: &Expr) -> Result<Value, String> {
        match e {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::StringLit(s) => Ok(Value::Str(s.clone())),
            Expr::Variable(n) => Ok(self.get_var(n)),
            Expr::FieldAccess(parent, fname) => match &**parent {
                Expr::Variable(n) => {
                    let pv = self.get_var_ref(n).ok_or_else(|| format!("Undefined variable '{}'", n))?;
                    if let Value::Record(rec) = pv {
                        for (fn_, fv) in &rec.fields {
                            if fn_.eq_ignore_ascii_case(fname) {
                                return Ok(fv.clone());
                            }
                        }
                        Err(format!("Field '{}' not found", fname))
                    } else {
                        Err("Field access on non-record".to_string())
                    }
                }
                _ => {
                    let pv = self.eval(parent)?;
                    if let Value::Record(rec) = pv {
                        for (fn_, fv) in rec.fields {
                            if fn_.eq_ignore_ascii_case(fname) {
                                return Ok(fv);
                            }
                        }
                        Err(format!("Field '{}' not found", fname))
                    } else {
                        Err("Field access on non-record".to_string())
                    }
                }
            },
            Expr::UnaryMinus(inner) => {
                let v = self.eval(inner)?;
                Ok(Value::Number(-v.as_number()?))
            }
            Expr::Not(inner) => {
                let v = self.eval(inner)?.as_number()?;
                Ok(Value::Number(!(v as i64) as f64))
            }
            Expr::BinaryOp(l, op, r) => {
                let lv = self.eval(l)?;
                let rv = self.eval(r)?;
                apply_binop(&lv, op, &rv)
            }
            Expr::ArrayOrCall(name, args) => {
                if let Some(v) = self.try_builtin(name, args)? {
                    return Ok(v);
                }
                if self.subs.contains_key(name) {
                    return self.invoke_sub(name, args);
                }
                let mut idxs = Vec::new();
                for a in args {
                    idxs.push(self.eval(a)?.as_number()? as usize);
                }
                let v = self.get_var_ref(name);
                if let Some(Value::Array(a)) = v {
                    let flat = flatten_index(&a.dims, &idxs)?;
                    Ok(a.data[flat].clone())
                } else {
                    Err(format!("Undefined array '{}'", name))
                }
            }
        }
    }
    fn try_builtin(&mut self, name: &str, args: &[Expr]) -> Result<Option<Value>, String> {
        let evaluated: Result<Vec<_>, _> = args.iter().map(|a| self.eval(a)).collect();
        let av: Vec<Value> = evaluated?;
        let v = match name {
            "LEN" => {
                let s = string_arg(&av, 0, "LEN")?;
                Value::Number(s.chars().count() as f64)
            }
            "MID$" | "MID" => {
                let s = string_arg(&av, 0, "MID$")?;
                let start = number_arg(&av, 1, "MID$")?;
                let len = if av.len() > 2 {
                    Some(count_arg(&av, 2, "MID$")?)
                } else {
                    None
                };
                let from = if start <= 1.0 { 0 } else { start as usize - 1 };
                let iter = s.chars().skip(from);
                let result_str: String = match len {
                    Some(l) => iter.take(l).collect(),
                    None => iter.collect(),
                };
                Value::Str(result_str)
            }
            "LEFT$" | "LEFT" => {
                let s = string_arg(&av, 0, "LEFT$")?;
                let n = count_arg(&av, 1, "LEFT$")?;
                Value::Str(s.chars().take(n).collect())
            }
            "RIGHT$" | "RIGHT" => {
                let s = string_arg(&av, 0, "RIGHT$")?;
                let n = count_arg(&av, 1, "RIGHT$")?;
                let chars_count = s.chars().count();
                let start = chars_count.saturating_sub(n);
                Value::Str(s.chars().skip(start).collect())
            }
            "INT" => Value::Number(number_arg(&av, 0, "INT")?.floor()),
            "FIX" => Value::Number(number_arg(&av, 0, "FIX")?.trunc()),
            "ABS" => Value::Number(number_arg(&av, 0, "ABS")?.abs()),
            "SQR" => Value::Number(number_arg(&av, 0, "SQR")?.sqrt()),
            "RND" => {
                if let Some(Value::Number(seed)) = av.first() {
                    if *seed < 0.0 {
                        self.rng_state = seed_from_number(*seed);
                    }
                }
                let mut x = self.rng_state;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.rng_state = x;
                Value::Number(x as f64 / u64::MAX as f64)
            }
            "STR$" => {
                let n = number_arg(&av, 0, "STR$")?;
                let s = if n >= 0.0 {
                    format!(" {}", Value::Number(n).to_display())
                } else {
                    Value::Number(n).to_display()
                };
                Value::Str(s)
            }
            "CSTR$" | "CSTR" => {
                Value::Str(Value::Number(number_arg(&av, 0, "CSTR$")?).to_display())
            }
            "CINT" | "CLNG" => Value::Number(number_arg(&av, 0, name)?.round()),
            "CSNG" | "CDBL" => Value::Number(number_arg(&av, 0, name)?),
            "VAL" => {
                let s = string_arg(&av, 0, "VAL")?;
                let trimmed = s.trim();
                let mut end = 0;
                let mut saw_dot = false;
                for (i, c) in trimmed.char_indices() {
                    if c.is_ascii_digit()
                        || (c == '.' && !saw_dot)
                        || (i == 0 && (c == '-' || c == '+'))
                    {
                        if c == '.' {
                            saw_dot = true;
                        }
                        end = i + c.len_utf8();
                    } else {
                        break;
                    }
                }
                let slice = &trimmed[..end];
                let n: f64 = if slice == "-" || slice == "+" || slice == "." {
                    0.0
                } else {
                    slice.parse().unwrap_or(0.0)
                };
                Value::Number(n)
            }
            "CHR$" => {
                let n = number_arg(&av, 0, "CHR$")? as u32;
                Value::Str(
                    std::char::from_u32(n)
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                )
            }
            "ASC" => {
                let s = string_arg(&av, 0, "ASC")?;
                let c = s.chars().next().ok_or("ASC: empty")?;
                Value::Number(c as u32 as f64)
            }
            "UCASE$" => Value::Str(string_arg(&av, 0, "UCASE$")?.to_uppercase()),
            "LCASE$" => Value::Str(string_arg(&av, 0, "LCASE$")?.to_lowercase()),
            "LTRIM$" | "LTRIM" => {
                Value::Str(string_arg(&av, 0, "LTRIM$")?.trim_start().to_string())
            }
            "RTRIM$" | "RTRIM" => Value::Str(string_arg(&av, 0, "RTRIM$")?.trim_end().to_string()),
            "TRIM$" | "TRIM" => Value::Str(string_arg(&av, 0, "TRIM$")?.trim().to_string()),
            "INSTR" => {
                let h = string_arg(&av, 0, "INSTR")?;
                let n = string_arg(&av, 1, "INSTR")?;
                Value::Number(match h.find(&n) {
                    Some(i) => (i + 1) as f64,
                    None => 0.0,
                })
            }
            "SIN" => Value::Number(number_arg(&av, 0, "SIN")?.sin()),
            "COS" => Value::Number(number_arg(&av, 0, "COS")?.cos()),
            "TAN" => Value::Number(number_arg(&av, 0, "TAN")?.tan()),
            "ATN" => Value::Number(number_arg(&av, 0, "ATN")?.atan()),
            "LOG" => Value::Number(number_arg(&av, 0, "LOG")?.ln()),
            "EXP" => Value::Number(number_arg(&av, 0, "EXP")?.exp()),
            "SGN" => {
                let n = number_arg(&av, 0, "SGN")?;
                Value::Number(if n > 0.0 {
                    1.0
                } else if n < 0.0 {
                    -1.0
                } else {
                    0.0
                })
            }
            "TIMER" => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap();
                Value::Number((now.as_secs() % 86400) as f64 + now.subsec_nanos() as f64 / 1e9)
            }
            "SPACE$" => {
                let n = count_arg(&av, 0, "SPACE$")?;
                Value::Str(" ".repeat(n))
            }
            "SPC" | "TAB" => {
                let n = count_arg(&av, 0, name)?;
                Value::Str(" ".repeat(n))
            }
            "STRING$" => {
                let n = count_arg(&av, 0, "STRING$")?;
                let c = match arg(&av, 1, "STRING$")? {
                    Value::Number(num) => std::char::from_u32(*num as u32).unwrap_or(' '),
                    Value::Str(s) => s.chars().next().unwrap_or(' '),
                    _ => ' ',
                };
                Value::Str(std::iter::repeat_n(c, n).collect())
            }
            "HEX$" => Value::Str(format!("{:X}", number_arg(&av, 0, "HEX$")? as i64)),
            "OCT$" => Value::Str(format!("{:o}", number_arg(&av, 0, "OCT$")? as i64)),
            _ => return Ok(None),
        };
        Ok(Some(v))
    }
    fn invoke_sub(&mut self, name: &str, args: &[Expr]) -> Result<Value, String> {
        let info = self
            .subs
            .get(name)
            .ok_or_else(|| format!("Undefined sub/function '{}'", name))?
            .clone_meta();
        if args.len() != info.params.len() {
            return Err(format!(
                "'{}' expects {} args, got {}",
                name,
                info.params.len(),
                args.len()
            ));
        }
        let mut arg_values: Vec<Value> = Vec::new();
        let mut byref_targets: Vec<Option<LValue>> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let p = &info.params[i];
            if p.by_ref {
                if let Some(lv) = expr_to_lvalue(a) {
                    let v = self.read_lvalue(&lv)?;
                    arg_values.push(v);
                    byref_targets.push(Some(lv));
                } else {
                    arg_values.push(self.eval(a)?);
                    byref_targets.push(None);
                }
            } else {
                arg_values.push(self.eval(a)?);
                byref_targets.push(None);
            }
        }
        let mut new_scope = Scope::new();
        for (p, v) in info.params.iter().zip(arg_values) {
            let coerced = if p.vtype.is_string() {
                Value::Str(v.as_string()?)
            } else if matches!(v, Value::Record(_) | Value::Array(_)) {
                v
            } else {
                Value::Number(v.as_number()?)
            };
            new_scope.vars.insert(p.name.clone(), coerced);
        }
        if info.is_function {
            let init = self.default_value_for_type(&info.ret_type);
            new_scope.vars.insert(name.to_string(), init);
        }
        self.scopes.push(new_scope);
        let result = self.exec_block(&info.body);
        let ret_val = if info.is_function {
            self.scopes
                .last()
                .unwrap()
                .vars
                .get(name)
                .cloned()
                .unwrap_or(Value::Number(0.0))
        } else {
            Value::Number(0.0)
        };
        let mut byref_vals = Vec::new();
        for p in &info.params {
            byref_vals.push(self.scopes.last().unwrap().vars.get(&p.name).cloned());
        }
        self.scopes.pop();
        match result? {
            Flow::Normal | Flow::ExitSub | Flow::Return => {}
            Flow::End => return Ok(ret_val),
            Flow::Goto(t) => return Err(format!("GOTO out of sub to '{}'", t)),
            _ => {}
        }
        for (i, target) in byref_targets.iter().enumerate() {
            if let Some(lv) = target {
                if let Some(v) = byref_vals[i].clone() {
                    self.assign(lv, v)?;
                }
            }
        }
        Ok(ret_val)
    }
}

fn arg<'a>(values: &'a [Value], idx: usize, name: &str) -> Result<&'a Value, String> {
    values
        .get(idx)
        .ok_or_else(|| format!("{}: missing argument {}", name, idx + 1))
}

fn data_item_to_value(item: &DataItem) -> Value {
    match item {
        DataItem::Number(n) => Value::Number(*n),
        DataItem::Str(s) => Value::Str(s.clone()),
    }
}

fn seed_from_number(seed: f64) -> u64 {
    let mixed = seed.to_bits() ^ 0x9E37_79B9_7F4A_7C15;
    if mixed == 0 {
        0x1234_5678
    } else {
        mixed
    }
}

fn seed_from_time() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x1234_5678);
    if nanos == 0 {
        0x1234_5678
    } else {
        nanos
    }
}

fn number_arg(values: &[Value], idx: usize, name: &str) -> Result<f64, String> {
    arg(values, idx, name)?.as_number()
}

fn string_arg(values: &[Value], idx: usize, name: &str) -> Result<String, String> {
    arg(values, idx, name)?.as_string()
}

fn count_arg(values: &[Value], idx: usize, name: &str) -> Result<usize, String> {
    let n = number_arg(values, idx, name)?;
    if n < 0.0 {
        return Err(format!("{}: negative count", name));
    }
    Ok(n as usize)
}

fn expr_to_lvalue(e: &Expr) -> Option<LValue> {
    match e {
        Expr::Variable(n) => Some(LValue::Var(n.clone())),
        Expr::ArrayOrCall(n, args) => Some(LValue::Index(n.clone(), args.clone())),
        Expr::FieldAccess(parent, fname) => {
            let p = expr_to_lvalue(parent)?;
            Some(LValue::Field(Box::new(p), fname.clone()))
        }
        _ => None,
    }
}

fn runtime_type_for_value(name: &str, v: &Value) -> VarType {
    match v {
        Value::Str(_) => VarType::Str,
        Value::Record(rec) => VarType::UserType(rec.type_name.clone()),
        Value::Array(arr) => arr.element_type.clone(),
        Value::Number(_) => infer_type_from_suffix(name),
    }
}

fn coerce_to_existing_shape(
    name: &str,
    existing: Option<&Value>,
    v: Value,
    inferred_type: &VarType,
) -> Result<Value, String> {
    match existing {
        Some(Value::Str(_)) => Ok(Value::Str(v.as_string()?)),
        Some(Value::Number(_)) => Ok(Value::Number(v.as_number()?)),
        Some(Value::Record(_)) => {
            if matches!(v, Value::Record(_)) {
                Ok(v)
            } else {
                Err("Type mismatch: expected record".to_string())
            }
        }
        Some(Value::Array(_)) => {
            if matches!(v, Value::Array(_)) {
                Ok(v)
            } else {
                Err("Type mismatch: expected array".to_string())
            }
        }
        None if name.ends_with('$') || inferred_type.is_string() => Ok(Value::Str(v.as_string()?)),
        None if matches!(v, Value::Record(_) | Value::Array(_)) => Ok(v),
        None => Ok(Value::Number(v.as_number()?)),
    }
}

fn type_size(vt: &VarType) -> usize {
    match vt {
        VarType::Integer | VarType::Long | VarType::Single | VarType::Double => 8,
        VarType::Str => 32,
        _ => 8,
    }
}

fn serialize(v: &Value, buf: &mut [u8]) {
    match v {
        Value::Number(n) => {
            let bytes = n.to_le_bytes();
            let len = bytes.len().min(buf.len());
            buf[..len].copy_from_slice(&bytes[..len]);
        }
        Value::Str(s) => {
            let bytes = s.as_bytes();
            let len = bytes.len().min(buf.len());
            buf[..len].copy_from_slice(&bytes[..len]);
            for b in &mut buf[len..] {
                *b = 0;
            }
        }
        Value::Record(rec) => {
            let mut offset = 0;
            for (_, fv) in &rec.fields {
                let fsize = match fv {
                    Value::Str(_) => 32,
                    _ => 8,
                };
                if offset + fsize > buf.len() {
                    break;
                }
                serialize(fv, &mut buf[offset..offset + fsize]);
                offset += fsize;
            }
        }
        _ => {}
    }
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Number(n) => *n != 0.0,
        Value::Str(s) => !s.is_empty(),
        _ => false,
    }
}

fn block_label(body: &[Stmt], target: &str) -> Option<usize> {
    for (i, s) in body.iter().enumerate() {
        match s {
            Stmt::Label(n) if n == target => return Some(i),
            Stmt::LineNumber(n) if format!("{}", n) == target => return Some(i),
            _ => {}
        }
    }
    None
}

fn flatten_index(dims: &[usize], idxs: &[usize]) -> Result<usize, String> {
    if dims.len() != idxs.len() {
        return Err("Array index count mismatch".to_string());
    }
    let mut flat = 0;
    let mut mult = 1;
    for i in (0..dims.len()).rev() {
        if idxs[i] >= dims[i] {
            return Err("Array index out of bounds".to_string());
        }
        flat += idxs[i] * mult;
        mult *= dims[i];
    }
    Ok(flat)
}

fn apply_binop(l: &Value, op: &BinOp, r: &Value) -> Result<Value, String> {
    if let (Value::Str(a), Value::Str(b)) = (l, r) {
        return match op {
            BinOp::Add => Ok(Value::Str(format!("{}{}", a, b))),
            BinOp::Eq => Ok(Value::Number(if a == b { -1.0 } else { 0.0 })),
            BinOp::NotEq => Ok(Value::Number(if a != b { -1.0 } else { 0.0 })),
            BinOp::Lt => Ok(Value::Number(if a < b { -1.0 } else { 0.0 })),
            BinOp::Le => Ok(Value::Number(if a <= b { -1.0 } else { 0.0 })),
            BinOp::Gt => Ok(Value::Number(if a > b { -1.0 } else { 0.0 })),
            BinOp::Ge => Ok(Value::Number(if a >= b { -1.0 } else { 0.0 })),
            _ => Err("Invalid string operation".to_string()),
        };
    }
    let a = l.as_number()?;
    let b = r.as_number()?;
    Ok(match op {
        BinOp::Add => Value::Number(a + b),
        BinOp::Sub => Value::Number(a - b),
        BinOp::Mul => Value::Number(a * b),
        BinOp::Div => {
            if b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Value::Number(a / b)
        }
        BinOp::IntDiv => {
            if b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Value::Number((a as i64 / b as i64) as f64)
        }
        BinOp::Mod => {
            if b == 0.0 {
                return Err("Division by zero".to_string());
            }
            Value::Number((a as i64 % b as i64) as f64)
        }
        BinOp::Pow => Value::Number(a.powf(b)),
        BinOp::Eq => Value::Number(if a == b { -1.0 } else { 0.0 }),
        BinOp::NotEq => Value::Number(if a != b { -1.0 } else { 0.0 }),
        BinOp::Lt => Value::Number(if a < b { -1.0 } else { 0.0 }),
        BinOp::Le => Value::Number(if a <= b { -1.0 } else { 0.0 }),
        BinOp::Gt => Value::Number(if a > b { -1.0 } else { 0.0 }),
        BinOp::Ge => Value::Number(if a >= b { -1.0 } else { 0.0 }),
        BinOp::And => Value::Number(((a as i64) & (b as i64)) as f64),
        BinOp::Or => Value::Number(((a as i64) | (b as i64)) as f64),
        BinOp::Xor => Value::Number(((a as i64) ^ (b as i64)) as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn run_source(src: &str, inputs: &[&str]) -> Result<Vec<String>, String> {
        let tokens = crate::lexer::tokenize(src)?;
        let program = crate::parser::parse(tokens)?;
        let (console_tx, console_rx) = mpsc::channel();
        let (input_tx, input_rx) = mpsc::channel();
        for input in inputs {
            input_tx.send((*input).to_string()).unwrap();
        }
        drop(input_tx);

        let result = run(
            program,
            console_tx,
            input_rx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(SharedGraphics::new())),
        );

        let mut lines = vec![String::new()];
        for msg in console_rx.try_iter() {
            match msg {
                ConsoleMsg::Print { text, newline } => {
                    lines.last_mut().unwrap().push_str(&text);
                    if newline {
                        lines.push(String::new());
                    }
                }
                ConsoleMsg::Clear => lines.clear(),
                ConsoleMsg::InputPrompt(_) | ConsoleMsg::End(_) => {}
            }
        }

        result.map(|_| lines.into_iter().filter(|line| !line.is_empty()).collect())
    }

    #[test]
    fn assigns_dim_string_without_suffix() {
        let lines = run_source("DIM S AS STRING\nS = \"OK\"\nPRINT S\n", &[]).unwrap();
        assert_eq!(lines, vec!["OK"]);
    }

    #[test]
    fn assigns_and_reads_string_record_field_without_suffix() {
        let lines = run_source(
            "TYPE PERSON\nNAME AS STRING\nEND TYPE\nDIM P AS PERSON\nP.NAME = \"ADA\"\nS$ = P.NAME\nPRINT S$\n",
            &[],
        )
        .unwrap();
        assert_eq!(lines, vec!["ADA"]);
    }

    #[test]
    fn input_uses_declared_string_type() {
        let lines = run_source("DIM S AS STRING\nINPUT S\nPRINT S\n", &["hello"]).unwrap();
        assert_eq!(lines, vec!["hello"]);
    }

    #[test]
    fn invalid_builtin_call_returns_error_instead_of_panicking() {
        let err = run_source("PRINT SIN()\n", &[]).unwrap_err();
        assert!(err.contains("SIN: missing argument 1"));
    }

    #[test]
    fn assigning_to_constant_is_an_error() {
        let err = run_source("CONST X = 1\nX = 2\n", &[]).unwrap_err();
        assert!(err.contains("Cannot assign to constant"));
    }

    #[test]
    fn for_loop_cannot_use_constant_counter() {
        let err = run_source("CONST X = 1\nFOR X = 1 TO 3\nNEXT X\n", &[]).unwrap_err();
        assert!(err.contains("Cannot assign to constant"));
    }

    #[test]
    fn read_consumes_data_and_restore_rewinds() {
        let lines = run_source(
            "DATA 10, \"ADA\"\nREAD N, NAME$\nPRINT N; \":\"; NAME$\nRESTORE\nREAD AGAIN\nPRINT AGAIN\n",
            &[],
        )
        .unwrap();
        assert_eq!(lines, vec!["10:ADA", "10"]);
    }

    #[test]
    fn restore_can_target_line_number() {
        let lines = run_source(
            "10 DATA 1\n20 DATA 2\nREAD A\nRESTORE 20\nREAD B\nPRINT A; \",\"; B\n",
            &[],
        )
        .unwrap();
        assert_eq!(lines, vec!["1,2"]);
    }

    #[test]
    fn read_past_data_returns_error() {
        let err = run_source("DATA 1\nREAD A, B\n", &[]).unwrap_err();
        assert!(err.contains("READ past end of DATA"));
    }

    #[test]
    fn common_classic_basic_syntax_runs() {
        let lines = run_source(
            "OPTION BASE 1\nDEFINT A-Z\nDECLARE SUB IGNORED()\nREDIM A(2)\nA(1) = 10\nA(2) = 20\nSWAP A(1), A(2)\nPRINT A(1); \",\"; A(2)\nN = 2\nON N GOTO 100, 200\n100 PRINT \"WRONG\": END\n200 LINE INPUT \"Name\"; NAME$\nPRINT NAME$\n",
            &["Ada, Lovelace"],
        )
        .unwrap();
        assert_eq!(lines, vec!["20,10", "Ada, Lovelace"]);
    }

    #[test]
    fn defstr_sets_default_string_variables() {
        let lines = run_source("DEFSTR A-Z\nA = \"OK\"\nPRINT A\n", &[]).unwrap();
        assert_eq!(lines, vec!["OK"]);
    }
}
