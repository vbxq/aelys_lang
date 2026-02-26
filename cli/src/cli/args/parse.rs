// hand-rolled recursive descent, clap felt overkill for this

use super::{Command, ParsedArgs};
use aelys_opt::OptimizationLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandName {
    Compile,
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
    opt_level: OptimizationLevel,
    output: Option<String>,
    emit_air: bool,
    emit_llvm_ir: bool,
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
            opt_level: OptimizationLevel::Standard,
            output: None,
            emit_air: false,
            emit_llvm_ir: false,
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

            if let Some(consumed_next) = self.parse_output_option(token_str)? {
                self.advance();
                if consumed_next {
                    self.advance();
                }
                continue;
            }

            if token_str == "--emit-air" {
                self.emit_air = true;
                self.advance();
                continue;
            }

            if token_str == "--emit-llvm-ir" {
                self.emit_llvm_ir = true;
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
            Some(CommandName::Compile) => {
                let path = self
                    .path
                    .ok_or_else(|| "missing file for compile".to_string())?;
                if self.emit_air && self.output.is_some() {
                    return Err("--emit-air and --output cannot be combined".to_string());
                }
                Command::Compile {
                    path,
                    output: self.output,
                    emit_air: self.emit_air,
                    emit_llvm_ir: self.emit_llvm_ir,
                }
            }
        };

        Ok(ParsedArgs {
            command,
            opt_level: self.opt_level,
            warning_flags: self.warning_flags,
        })
    }

    fn finish_help(self) -> ParsedArgs {
        ParsedArgs {
            command: Command::Help,
            opt_level: OptimizationLevel::Standard,
            warning_flags: Vec::new(),
        }
    }

    fn finish_version(self) -> ParsedArgs {
        ParsedArgs {
            command: Command::Version,
            opt_level: OptimizationLevel::Standard,
            warning_flags: Vec::new(),
        }
    }

    fn consume_positional(&mut self, token: &str) -> Result<(), String> {
        match self.command {
            None => {
                return Err(format!(
                    "unexpected argument: {}. Use 'aelys compile <file>' to compile.",
                    token
                ));
            }
            Some(CommandName::Compile) => {
                if self.path.is_none() {
                    self.path = Some(token.to_string());
                } else {
                    return Err(format!("unexpected argument for compile: {}", token));
                }
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
            "compile" => Some(CommandName::Compile),
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
