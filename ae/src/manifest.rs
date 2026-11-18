use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use aelys_common::Diagnostic;
use aelys_driver::RuntimeVariant;
use aelys_opt::OptimizationLevel;
use aelys_syntax::{Source, Span};
use serde::Deserialize;
use toml::{Spanned, Value};

use crate::error::{Error, Result, closest, error};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawManifest {
    package: RawPackage,
    dependencies: Option<Spanned<Value>>,
    dev_dependencies: Option<Spanned<Value>>,
    #[serde(default)]
    native: RawNative,
    lib: Option<Spanned<Value>>,
    #[serde(default)]
    bin: Vec<BinDecl>,
    test: Option<Spanned<Value>>,
    #[serde(default)]
    profile: RawProfiles,
    workspace: Option<Spanned<Value>>,
    patch: Option<Spanned<Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawPackage {
    name: Spanned<String>,
    version: Spanned<String>,
    default_run: Option<Spanned<String>>,
    #[serde(rename = "description")]
    _description: Option<String>,
    #[serde(rename = "license")]
    _license: Option<String>,
    #[serde(rename = "authors")]
    _authors: Option<Vec<String>>,
    #[serde(rename = "repository")]
    _repository: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawNative {
    #[serde(default)]
    pkg_config: Vec<Spanned<String>>,
    #[serde(default)]
    libs: Vec<Spanned<String>>,
    #[serde(default)]
    search: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawProfiles {
    dev: Option<RawProfile>,
    release: Option<RawProfile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawProfile {
    opt_level: Option<Spanned<i64>>,
    runtime: Option<Spanned<String>>,
}

pub struct Manifest {
    pub name: String,
    pub version: semver::Version,
    pub default_run: Option<Spanned<String>>,
    pub bins: Vec<BinDecl>,
    pub native: Native,
    pub dev: Profile,
    pub release: Profile,
    pub origin: Origin,
}

pub struct Origin {
    source: Arc<Source>,
    header: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinDecl {
    pub name: Spanned<String>,
    pub path: Option<String>,
}

#[derive(Default)]
pub struct Native {
    pub pkg_config: Vec<Spanned<String>>,
    pub libs: Vec<String>,
    pub search: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct Profile {
    pub opt_level: OptimizationLevel,
    pub runtime: RuntimeVariant,
}

const RESERVED: &[&str] = &[
    "std", "crate", "self", "super", "Self", "and", "as", "break", "catch", "const", "continue",
    "discard", "dyn", "else", "enum", "extern", "false", "fn", "for", "from", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "needs", "nogc", "not", "null", "or", "pub",
    "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where",
    "while",
];

pub fn check_name(name: &str) -> std::result::Result<(), String> {
    let Some(first) = name.chars().next() else {
        return Err("a package name cannot be empty".to_string());
    };
    if first.is_ascii_digit() {
        return Err("a package name cannot start with a digit".to_string());
    }
    if first == '-' {
        return Err("a package name cannot start with `-`".to_string());
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
    {
        return Err(format!(
            "the character `{bad}` is not allowed in a package name"
        ));
    }
    if RESERVED.contains(&name) {
        return Err(format!("`{name}` is a reserved name"));
    }
    Ok(())
}

impl Manifest {
    pub fn parse(text: &str, path: &Path) -> Result<Manifest> {
        let origin = Origin {
            source: Source::new("Ae.toml", text),
            header: format!("failed to parse manifest `{}`", path.display()),
        };
        let raw: RawManifest = toml::from_str(text).map_err(|err| origin.toml_error(&err))?;

        let unsupported = [
            ("dependencies", raw.dependencies, true),
            ("dev-dependencies", raw.dev_dependencies, true),
            ("lib", raw.lib, false),
            ("test", raw.test, false),
            ("workspace", raw.workspace, false),
            ("patch", raw.patch, false),
        ];
        for (key, value, empty_is_fine) in unsupported {
            let Some(value) = value else { continue };
            let empty = value
                .get_ref()
                .as_table()
                .is_some_and(|table| table.is_empty());
            if empty_is_fine && empty {
                continue;
            }
            let mut diagnostic = origin.at(
                value.span(),
                format!("`{key}` is not supported by this version of ae"),
            );
            diagnostic.add_note("ae builds single-package projects without dependencies for now");
            return Err(diagnostic.into());
        }

        let name = raw.package.name;
        if let Err(reason) = check_name(name.get_ref()) {
            return Err(origin
                .at(
                    name.span(),
                    format!("invalid package name `{}`: {reason}", name.get_ref()),
                )
                .into());
        }
        let version = semver::Version::parse(raw.package.version.get_ref()).map_err(|_| {
            let mut diagnostic = origin.at(
                raw.package.version.span(),
                format!(
                    "`{}` is not a semver version",
                    raw.package.version.get_ref()
                ),
            );
            diagnostic.add_help("write three components, e.g. `0.1.0`");
            diagnostic
        })?;

        for lib in &raw.native.libs {
            let value = lib.get_ref();
            if value.is_empty() || value.starts_with('-') || value.contains(char::is_whitespace) {
                let mut diagnostic =
                    origin.at(lib.span(), format!("invalid native library name `{value}`"));
                diagnostic.add_help("write the bare library name, e.g. `m` for libm");
                return Err(diagnostic.into());
            }
        }

        for bin in &raw.bin {
            if let Err(reason) = check_name(bin.name.get_ref()) {
                let reason = reason.replace("package name", "target name");
                return Err(origin
                    .at(
                        bin.name.span(),
                        format!("invalid bin name `{}`: {reason}", bin.name.get_ref()),
                    )
                    .into());
            }
        }

        let mut dev = Profile {
            opt_level: OptimizationLevel::None,
            runtime: RuntimeVariant::Rc,
        };
        let mut release = Profile {
            opt_level: OptimizationLevel::Aggressive,
            runtime: RuntimeVariant::Rc,
        };
        if let Some(raw) = raw.profile.dev {
            origin.apply_profile(raw, &mut dev)?;
        }
        if let Some(raw) = raw.profile.release {
            origin.apply_profile(raw, &mut release)?;
        }

        Ok(Manifest {
            name: name.into_inner(),
            version,
            default_run: raw.package.default_run,
            bins: raw.bin,
            native: Native {
                pkg_config: raw.native.pkg_config,
                libs: raw
                    .native
                    .libs
                    .into_iter()
                    .map(Spanned::into_inner)
                    .collect(),
                search: raw.native.search,
            },
            dev,
            release,
            origin,
        })
    }
}

impl Origin {
    fn apply_profile(&self, raw: RawProfile, profile: &mut Profile) -> Result<()> {
        if let Some(level) = raw.opt_level {
            profile.opt_level = match *level.get_ref() {
                0 => OptimizationLevel::None,
                1 => OptimizationLevel::Basic,
                2 => OptimizationLevel::Standard,
                3 => OptimizationLevel::Aggressive,
                other => {
                    return Err(self
                        .at(
                            level.span(),
                            format!("opt-level must be 0, 1, 2 or 3, not {other}"),
                        )
                        .into());
                }
            };
        }
        if let Some(runtime) = raw.runtime {
            profile.runtime = RuntimeVariant::parse(runtime.get_ref()).ok_or_else(|| {
                let mut diagnostic = self.at(
                    runtime.span(),
                    format!("unknown runtime `{}`", runtime.get_ref()),
                );
                diagnostic.add_help("expected `rc`, `rc+cycles` or `leak`");
                diagnostic
            })?;
        }
        Ok(())
    }

    pub fn at(&self, range: Range<usize>, label: impl Into<String>) -> Diagnostic {
        error(self.header.clone()).with_primary_label(
            self.source.clone(),
            self.span(range),
            Some(label.into()),
        )
    }

    pub fn error_at(&self, range: Range<usize>, message: impl Into<String>) -> Diagnostic {
        error(message).with_primary_label(self.source.clone(), self.span(range), None)
    }

    fn span(&self, range: Range<usize>) -> Span {
        let first_line_end = self.source.content[range.start..]
            .find('\n')
            .map_or(range.end, |offset| range.start + offset);
        let (line, column) = self.source.line_col_at_offset(range.start);
        Span::new(range.start, range.end.min(first_line_end), line, column)
    }

    fn toml_error(&self, err: &toml::de::Error) -> Error {
        let range = err.span().unwrap_or(0..0);
        let message = err.message().trim();
        let Some((field, expected)) = unknown_field(message) else {
            let mut lines = message.lines();
            let mut diagnostic = self.at(range, lines.next().unwrap_or("invalid TOML"));
            for line in lines {
                diagnostic.add_note(line.trim());
            }
            return diagnostic.into();
        };
        let mut diagnostic = self.at(range, format!("unknown field `{field}`"));
        if let Some(best) = closest(field, expected.iter().copied()) {
            diagnostic.add_help(format!("did you mean `{best}`?"));
        }
        if !expected.is_empty() {
            let listed: Vec<String> = expected.iter().map(|name| format!("`{name}`")).collect();
            diagnostic.add_note(format!("expected one of {}", listed.join(", ")));
        }
        diagnostic.into()
    }
}

fn unknown_field(message: &str) -> Option<(&str, Vec<&str>)> {
    let rest = message.strip_prefix("unknown field `")?;
    let (field, rest) = rest.split_once('`')?;
    let expected = rest.split('`').skip(1).step_by(2).collect();
    Some((field, expected))
}
