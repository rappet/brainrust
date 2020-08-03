//! # brainrust - A branf*ck interpreter in rust

extern crate serde;
extern crate serde_yaml;

use serde::{Deserialize, Serialize};

use core::fmt;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt::{Debug, Display, Formatter, Write};
use std::io;

pub struct Engine<R, W> {
    tape: HashMap<i32, u8>,
    pointer: i32,
    reader: R,
    writer: W,
}

impl<R, W> Engine<R, W>
    where
        R: io::Read,
        W: io::Write,
{
    pub fn new(reader: R, writer: W) -> Engine<R, W> {
        Engine {
            tape: HashMap::new(),
            pointer: 0,
            reader,
            writer,
        }
    }

    pub fn move_tape(&mut self, amount: i32) {
        self.pointer += amount;
    }

    pub fn get(&self) -> u8 {
        *self.tape.get(&self.pointer).unwrap_or(&0)
    }

    pub fn get_rel(&self, offset: i32) -> u8 {
        *self.tape.get(&(self.pointer + offset)).unwrap_or(&0)
    }

    pub fn set(&mut self, value: u8) {
        self.tape.insert(self.pointer, value);
    }

    pub fn set_rel(&mut self, value: u8, offset: i32) {
        self.tape.insert(self.pointer + offset, value);
    }

    pub fn write(&mut self, value: u8) {
        self.writer.write(&[value]).unwrap();
        self.writer.flush().unwrap();
    }

    pub fn read(&mut self) -> u8 {
        let mut value = [0u8];
        self.reader.read_exact(&mut value[..]).unwrap();
        value[0]
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Command {
    IncremenentPointer,
    DecrementPointer,
    IncrementValue,
    DecrementValue,
    Print,
    Read,
    /// [
    Jump,
    /// ]
    Target,
}

impl TryFrom<char> for Command {
    type Error = io::Error;

    fn try_from(c: char) -> io::Result<Command> {
        Ok(match c {
            '>' => Command::IncremenentPointer,
            '<' => Command::DecrementPointer,
            '+' => Command::IncrementValue,
            '-' => Command::DecrementValue,
            '.' => Command::Print,
            ',' => Command::Read,
            '[' => Command::Jump,
            ']' => Command::Target,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        })
    }
}

impl Into<char> for Command {
    fn into(self) -> char {
        match self {
            Command::IncremenentPointer => '>',
            Command::DecrementPointer => '<',
            Command::IncrementValue => '+',
            Command::DecrementValue => '-',
            Command::Print => '.',
            Command::Read => ',',
            Command::Jump => '[',
            Command::Target => ']',
        }
    }
}

impl Display for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_char((*self).into())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OptimizedCommand {
    MovePointer(i32),
    AddValue(u8),
    SetValue(u8),
    Print,
    Read,
    List(Vec<OptimizedCommand>),
    Jump(Box<OptimizedCommand>),
    AddTo(i32),
}

impl OptimizedCommand {
    pub fn build<I: Iterator<Item=Command>>(
        it: &mut I,
        in_loop: bool,
    ) -> io::Result<OptimizedCommand> {
        let mut commands = Vec::new();
        while let Some(command) = it.next() {
            commands.push(match command {
                Command::IncremenentPointer => OptimizedCommand::MovePointer(1),
                Command::DecrementPointer => OptimizedCommand::MovePointer(-1),
                Command::IncrementValue => OptimizedCommand::AddValue(1),
                Command::DecrementValue => OptimizedCommand::AddValue(255),
                Command::Print => OptimizedCommand::Print,
                Command::Read => OptimizedCommand::Read,
                Command::Jump => {
                    OptimizedCommand::Jump(Box::new(OptimizedCommand::build(it, true)?))
                }
                Command::Target => {
                    if in_loop {
                        break;
                    } else {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                }
            })
        }
        Ok(OptimizedCommand::List(commands))
    }

    fn execute(&self, engine: &mut Engine<impl io::Read, impl io::Write>) {
        match self {
            OptimizedCommand::MovePointer(offset) => engine.move_tape(*offset),
            OptimizedCommand::AddValue(summand) => engine.set(engine.get().wrapping_add(*summand)),
            OptimizedCommand::SetValue(value) => engine.set(*value),
            OptimizedCommand::Print => engine.write(engine.get()),
            OptimizedCommand::Read => {
                let v = engine.read();
                engine.set(v);
            }
            OptimizedCommand::List(v) => v.iter().for_each(|c| c.execute(engine)),
            OptimizedCommand::Jump(c) => {
                while engine.get() != 0 {
                    c.execute(engine)
                }
            }
            OptimizedCommand::AddTo(d) => {
                engine.set_rel(engine.get() + engine.get_rel(*d), *d);
            }
        }
    }

    fn optimize(self) -> Option<OptimizedCommand> {
        match self {
            OptimizedCommand::MovePointer(0) => None,
            OptimizedCommand::AddValue(0) => None,
            OptimizedCommand::List(v) if v.is_empty() => None,
            OptimizedCommand::List(v) if !v.is_empty() => {
                let commands = OptimizedCommand::optimize_vec(v);
                if commands.is_empty() {
                    None
                } else if commands.len() == 1 {
                    Some(commands.get(0).unwrap().to_owned())
                } else {
                    Some(OptimizedCommand::List(commands))
                }
            }
            OptimizedCommand::Jump(c) => OptimizedCommand::optimize_jump(*c),
            _ => Some(self),
        }
    }

    fn optimize_vec(v: Vec<OptimizedCommand>) -> Vec<OptimizedCommand> {
        let mut out = Vec::new();
        let mut holding = None;
        for command in v {
            match (holding.clone(), command) {
                (None, command) => holding = Some(command),
                (_, OptimizedCommand::AddValue(0)) => {}
                (_, OptimizedCommand::MovePointer(0)) => {}
                (Some(OptimizedCommand::SetValue(a)), OptimizedCommand::AddValue(b)) => {
                    holding = Some(OptimizedCommand::SetValue(a.wrapping_add(b)))
                }
                (Some(OptimizedCommand::SetValue(_)), OptimizedCommand::SetValue(v)) => {
                    holding = Some(OptimizedCommand::SetValue(v))
                }
                (Some(OptimizedCommand::AddValue(a)), OptimizedCommand::AddValue(b)) => {
                    holding = OptimizedCommand::AddValue(a.wrapping_add(b)).optimize()
                }
                (Some(OptimizedCommand::MovePointer(a)), OptimizedCommand::MovePointer(b)) => {
                    holding = OptimizedCommand::MovePointer(a.wrapping_add(b)).optimize()
                }
                (Some(old), command) => {
                    if let Some(optimized) = old.optimize() {
                        out.push(optimized);
                    }
                    holding = Some(command);
                }
            }
        }
        if let Some(command) = holding.and_then(|c| c.optimize()) {
            out.push(command);
        }
        out = {
            let mut compacted = Vec::new();
            for command in out {
                if let OptimizedCommand::List(mut v) = command {
                    compacted.append(&mut v);
                } else {
                    compacted.push(command);
                }
            }
            compacted
        };
        out
    }

    fn optimize_jump(inner: OptimizedCommand) -> Option<OptimizedCommand> {
        if let Some(inner) = inner.optimize() {
            match inner {
                OptimizedCommand::AddValue(1) => Some(OptimizedCommand::SetValue(0)),
                OptimizedCommand::AddValue(255) => Some(OptimizedCommand::SetValue(0)),
                OptimizedCommand::List(v) => match v.as_slice() {
                    [OptimizedCommand::AddValue(255), OptimizedCommand::MovePointer(a), OptimizedCommand::AddValue(1), OptimizedCommand::MovePointer(b)]
                    if *a == -*b =>
                        {
                            Some(OptimizedCommand::List(vec![OptimizedCommand::AddTo(*a), OptimizedCommand::SetValue(0)]))
                        }
                    [OptimizedCommand::AddValue(255), OptimizedCommand::MovePointer(a), OptimizedCommand::AddValue(1), OptimizedCommand::MovePointer(b), OptimizedCommand::AddValue(1), OptimizedCommand::MovePointer(s)]
                    if *a + *b == -*s => {
                        Some(OptimizedCommand::List(vec![
                            OptimizedCommand::AddTo(*a),
                            OptimizedCommand::AddTo(*a+*b),
                            OptimizedCommand::SetValue(0),
                        ]))
                    }
                    _ => Some(OptimizedCommand::Jump(Box::new(OptimizedCommand::List(v)))),
                },
                _ => Some(OptimizedCommand::Jump(Box::new(inner))),
            }
        } else {
            None
        }
    }

    pub fn optimize_pair(self, other: Self) -> Option<OptimizedCommand> {
        Some(match (self, other) {
            (OptimizedCommand::AddValue(a), OptimizedCommand::AddValue(b)) => {
                OptimizedCommand::AddValue(a.wrapping_add(b))
            }
            (OptimizedCommand::MovePointer(a), OptimizedCommand::MovePointer(b)) => {
                OptimizedCommand::MovePointer(a.wrapping_add(b))
            }
            _ => return None,
        })
    }
}

fn main() {
    let path = std::env::args().nth(1).expect(&format!("usage: {} <file>", std::env::args().nth(0).unwrap()));
    let source = std::fs::read_to_string(path).unwrap();
    let mut program: OptimizedCommand = OptimizedCommand::build(
            &mut source
            .chars()
            .filter_map(|c| Command::try_from(c).ok()),
        false,
    )
        .unwrap();
    //println!("{:?}", program);
    program = program.optimize().and_then(OptimizedCommand::optimize).unwrap();
    println!("{}", serde_yaml::to_string(&program).unwrap());
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut engine = Engine::new(stdin.lock(), stdout.lock());
    program.execute(&mut engine);
}
