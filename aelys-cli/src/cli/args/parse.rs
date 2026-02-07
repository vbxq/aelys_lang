// hand-rolled recursive descent, clap felt overkill for this

use super::{Command, ParsedArgs};
use aelys_opt::OptimizationLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandName {
    Run,
    Compile,
    Asm,
    Repl,
    Help,
    Version,
}

pub fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let parser = Parser::new(args);
    parser.parse()
}

struct Parser<'a> {
    tokens: Vec<&'a str>,
    index: usize,
    command: Option<CommandName>,
    path: Option<String>,
    program_args: Vec<String>,
    vm_args: Vec<String>,
    opt_level: OptimizationLevel,
    output: Option<String>,
    stdout: bool,
    warning_flags: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(args: &'a [String]) -> Self {
        let tokens = args.iter().skip(1).map(|s| s.as_str()).collect();
        Self {
            tokens,
            index: 0,
            command: None,
            path: None,
            program_args: Vec::new(),
            vm_args: Vec::new(),
            opt_level: OptimizationLevel::Standard,
            output: None,
            stdout: false,
            warning_flags: Vec::new(),
        }
    }

    fn parse(mut self) -> Result<ParsedArgs, String> {
        while self.index < self.tokens.len() {
            let token = self.tokens[self.index].to_string();
            let token_str = token.as_str();

            if self.is_help(token_str) {
                self.advance();
                return Ok(self.finish_help());
            }

            if self.is_version(token_str) {
                self.advance();
                return Ok(self.finish_version());
            }

            if let Some((level, consumed_next)) = self.parse_opt(token_str)? {
                self.opt_level = level;
                self.advance();
                if consumed_next {
                    self.advance();
                }
                continue;
            }

            if let Some((vm_arg, consumed_next)) = self.parse_vm_arg(token_str)? {
                self.vm_args.push(vm_arg);
                self.advance();
                if consumed_next {
                    self.advance();
                }
                continue;
            }

            if let Some(consumed_next) = self.parse_output_option(token_str)? {
                self.advance();
                if consumed_next {
                    self.advance();
                }
                continue;
            }

            if self.is_stdout(token_str) {
                self.stdout = true;
                self.advance();
                continue;
            }

            if let Some((wflag, consumed)) = self.parse_warning_flag(token_str)? {
                self.warning_flags.push(wflag);
                self.advance();
                if consumed {
                    self.advance();
                }
                continue;
            }

            if let Some(cmd) = self.parse_command(token_str)
                && self.command.is_none()
            {
                self.command = Some(cmd);
                self.advance();
                continue;
            }

            if token_str.starts_with('-') {
                if matches!(self.command, Some(CommandName::Run)) && self.path.is_some() {
                    self.program_args.push(token);
                    self.advance();
                    continue;
                }
                return Err(format!("unknown flag: {}", token_str));
            }

            self.consume_positional(token_str)?;
            self.advance();
        }

        self.finish()
    }

    fn finish(self) -> Result<ParsedArgs, String> {
        let command = match self.command {
            None => Command::Help,
            Some(CommandName::Help) => Command::Help,
            Some(CommandName::Version) => Command::Version,
            Some(CommandName::Repl) => {
                if self.path.is_some() || !self.program_args.is_empty() {
                    return Err("repl does not accept a path or arguments".to_string());
                }
                if self.output.is_some() || self.stdout {
                    return Err("repl does not accept output flags".to_string());
                }
                Command::Repl
            }
            Some(CommandName::Run) => {
                let path = self
                    .path
                    .ok_or_else(|| "missing file for run".to_string())?;
                if self.output.is_some() || self.stdout {
                    return Err("output flags are only supported for compile or asm".to_string());
                }
                Command::Run {
                    path,
                    program_args: self.program_args,
                }
            }
            Some(CommandName::Compile) => {
                let path = self
                    .path
                    .ok_or_else(|| "missing file for compile".to_string())?;
                if !self.program_args.is_empty() {
                    return Err("compile does not accept extra arguments".to_string());
                }
                if self.stdout {
                    return Err("compile does not support --stdout".to_string());
                }
                Command::Compile {
                    path,
                    output: self.output,
                }
            }
            Some(CommandName::Asm) => {
                let path = self
                    .path
                    .ok_or_else(|| "missing file for asm".to_string())?;
                if !self.program_args.is_empty() {
                    return Err("asm does not accept extra arguments".to_string());
                }
                Command::Asm {
                    path,
                    output: self.output,
                    stdout: self.stdout,
                }
            }
        };

        Ok(ParsedArgs {
            command,
            vm_args: self.vm_args,
            opt_level: self.opt_level,
            warning_flags: self.warning_flags,
        })
    }

    fn finish_help(self) -> ParsedArgs {
        ParsedArgs {
            command: Command::Help,
            vm_args: Vec::new(),
            opt_level: OptimizationLevel::Standard,
            warning_flags: Vec::new(),
        }
    }

    fn finish_version(self) -> ParsedArgs {
        ParsedArgs {
            command: Command::Version,
            vm_args: Vec::new(),
            opt_level: OptimizationLevel::Standard,
            warning_flags: Vec::new(),
        }
    }

    fn consume_positional(&mut self, token: &str) -> Result<(), String> {
        match self.command {
            None => {
                self.command = Some(CommandName::Run);
                self.path = Some(token.to_string());
            }
            Some(CommandName::Run) => {
                if self.path.is_none() {
                    self.path = Some(token.to_string());
                } else {
                    self.program_args.push(token.to_string());
                }
            }
            Some(CommandName::Compile) => {
                if self.path.is_none() {
                    self.path = Some(token.to_string());
                } else {
                    return Err(format!("unexpected argument for compile: {}", token));
                }
            }
            Some(CommandName::Asm) => {
                if self.path.is_none() {
                    self.path = Some(token.to_string());
                } else {
                    return Err(format!("unexpected argument for asm: {}", token));
                }
            }
            Some(CommandName::Repl) => {
                return Err(format!("unexpected argument for repl: {}", token));
            }
            Some(CommandName::Version) => {
                return Err(format!("unexpected argument for version: {}", token));
            }
            Some(CommandName::Help) => {}
        }
        Ok(())
    }

    fn parse_command(&self, token: &str) -> Option<CommandName> {
        match token {
            "run" => Some(CommandName::Run),
            "compile" => Some(CommandName::Compile),
            "asm" => Some(CommandName::Asm),
            "repl" => Some(CommandName::Repl),
            "help" => Some(CommandName::Help),
            "version" => Some(CommandName::Version),
            _ => None,
        }
    }

    fn parse_opt(&self, token: &str) -> Result<Option<(OptimizationLevel, bool)>, String> {
        if token == "-O" {
            let next = self
                .peek_next()
                .ok_or_else(|| "missing value for -O".to_string())?;
            let level = OptimizationLevel::parse(next)
                .ok_or_else(|| format!("invalid optimization level: {}", next))?;
            return Ok(Some((level, true)));
        }
        if let Some(rest) = token.strip_prefix("-O") {
            if rest.is_empty() {
                return Err("missing value for -O".to_string());
            }
            let level = OptimizationLevel::parse(rest)
                .ok_or_else(|| format!("invalid optimization level: {}", rest))?;
            return Ok(Some((level, false)));
        }
        Ok(None)
    }

    fn parse_vm_arg(&self, token: &str) -> Result<Option<(String, bool)>, String> {
        if token == "--dev" {
            return Ok(Some((token.to_string(), false)));
        }
        if token.starts_with("--allow-caps=") || token.starts_with("--deny-caps=") {
            return Ok(Some((token.to_string(), false)));
        }
        if token == "--allow-caps" || token == "--deny-caps" {
            let next = self
                .peek_next()
                .ok_or_else(|| format!("missing value for {}", token))?;
            return Ok(Some((format!("{}={}", token, next), true)));
        }
        if token.starts_with("-ae.") || token.starts_with("--ae-") {
            return Ok(Some((token.to_string(), false)));
        }
        Ok(None)
    }

    fn parse_warning_flag(&self, token: &str) -> Result<Option<(String, bool)>, String> {
        // -Wall, -Wno-inline, -Werror, etc
        if let Some(rest) = token.strip_prefix("-W") {
            if rest.is_empty() {
                return Err("-W requires a category (e.g. -Wall, -Werror)".into());
            }
            return Ok(Some((rest.to_string(), false)));
        }

        // --warn=error, --warn=all
        if let Some(rest) = token.strip_prefix("--warn=") {
            if rest.is_empty() {
                return Err("--warn requires a value".into());
            }
            return Ok(Some((rest.to_string(), false)));
        }

        if token == "--warn" {
            let next = self.peek_next().ok_or("--warn requires a value")?;
            return Ok(Some((next.to_string(), true)));
        }

        Ok(None)
    }

    fn is_help(&self, token: &str) -> bool {
        matches!(token, "-h" | "--help")
    }

    fn is_version(&self, token: &str) -> bool {
        matches!(token, "-v" | "--version")
    }

    fn is_stdout(&self, token: &str) -> bool {
        token == "--stdout"
    }

    fn parse_output_option(&mut self, token: &str) -> Result<Option<bool>, String> {
        if token == "-o" || token == "--output" {
            let next = self
                .peek_next()
                .ok_or_else(|| format!("missing value for {}", token))?;
            self.output = Some(next.to_string());
            return Ok(Some(true));
        }
        Ok(None)
    }

    fn peek_next(&self) -> Option<&str> {
        self.tokens.get(self.index + 1).copied()
    }

    fn advance(&mut self) {
        self.index = self.index.saturating_add(1);
    }
}
