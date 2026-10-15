//! product new — CA product scaffolder (REQ-P3-01).
//!
//! Generates a full CA-standard Rust workspace at `~/Product/<slug>` (or `--dir`).

use crate::output;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Valid slug: lowercase, alphanumeric + hyphen, no leading/trailing hyphen, non-empty.
pub fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        bail!("slug must not be empty");
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        bail!("slug must not start or end with a hyphen: {slug}");
    }
    if !slug
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("slug must be lowercase alphanumeric with hyphens only: {slug}");
    }
    Ok(())
}

/// Template kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Desktop,
    Cli,
    Service,
}

impl Template {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "desktop" => Ok(Template::Desktop),
            "cli" => Ok(Template::Cli),
            "service" => Ok(Template::Service),
            other => bail!("unknown template '{other}' (expected desktop|cli|service)"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Template::Desktop => "desktop",
            Template::Cli => "cli",
            Template::Service => "service",
        }
    }
}

/// Result of a scaffold run (used for JSON output).
pub struct ScaffoldResult {
    pub slug: String,
    pub path: PathBuf,
    pub template: String,
    pub files: Vec<String>,
}

/// Run the scaffolder.
pub fn run(
    slug: &str,
    template: &str,
    dir: &Option<PathBuf>,
    force: bool,
    check: bool,
) -> Result<ScaffoldResult> {
    validate_slug(slug)?;
    let template = Template::parse(template)?;

    let target = match dir {
        Some(d) => d.clone(),
        None => dirs::home_dir()
            .context("cannot determine home directory")?
            .join("Product")
            .join(slug),
    };

    // Refuse to clobber a non-empty directory unless --force.
    if target.exists() {
        let non_empty = std::fs::read_dir(&target)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
        if non_empty && !force {
            bail!(
                "target directory already exists and is not empty: {} (use --force to overwrite)",
                target.display()
            );
        }
    }
    std::fs::create_dir_all(&target)
        .with_context(|| format!("cannot create {}", target.display()))?;

    let files = plan(slug, template);
    for (rel, content) in &files {
        let path = target.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = content.replace("__SLUG__", slug);
        std::fs::write(&path, body).with_context(|| format!("cannot write {}", path.display()))?;
    }

    if check {
        cargo_check(&target)?;
    }

    Ok(ScaffoldResult {
        slug: slug.to_string(),
        path: target,
        template: template.as_str().to_string(),
        files: files.into_iter().map(|(rel, _)| rel).collect(),
    })
}

/// Run `cargo check` inside the generated project. Hard gate: bail on failure.
fn cargo_check(dir: &Path) -> Result<()> {
    let out = Command::new("cargo")
        .arg("check")
        .current_dir(dir)
        .output()
        .context("failed to invoke cargo check")?;
    if !out.status.success() {
        bail!(
            "cargo check failed for generated project:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// Print a friendly tree summary (TTY).
pub fn print_summary(res: &ScaffoldResult) {
    output::success(&format!("Scaffolded {} ({})", res.slug, res.template));
    output::kv("path", &res.path.display().to_string());
    output::kv("template", &res.template);
    println!("\n  {}/", res.slug);
    for f in &res.files {
        println!("    {f}");
    }
    println!("\n  Next steps:");
    println!("    cd {}", res.path.display());
    println!("    cargo build");
}

/// Print JSON summary (pipe).
pub fn print_json(res: &ScaffoldResult) -> Result<()> {
    let v = serde_json::json!({
        "slug": res.slug,
        "path": res.path.display().to_string(),
        "template": res.template,
        "files": res.files,
    });
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

/// The full file plan: (relative path, content template).
fn plan(slug_unused: &str, template: Template) -> Vec<(String, String)> {
    let _ = slug_unused;
    vec![
        (
            "app.toml".into(),
            APP_TOML.replace("__TEMPLATE__", template.as_str()),
        ),
        (".caframework.yaml".into(), CAFRAMEWORK_YAML.into()),
        ("Cargo.toml".into(), WORKSPACE_CARGO.into()),
        (".gitignore".into(), GITIGNORE.into()),
        ("rustfmt.toml".into(), RUSTFMT_TOML.into()),
        ("clippy.toml".into(), CLIPPY_TOML.into()),
        (
            "README.md".into(),
            README.replace("__TEMPLATE__", template.as_str()),
        ),
        ("crates/caf-core/Cargo.toml".into(), CORE_CARGO.into()),
        ("crates/caf-core/src/lib.rs".into(), CORE_LIB.into()),
        ("crates/caf-app/Cargo.toml".into(), APP_CARGO.into()),
        ("crates/caf-app/src/main.rs".into(), APP_MAIN.into()),
        ("crates/caf-cli/Cargo.toml".into(), CLI_CARGO.into()),
        ("crates/caf-cli/src/lib.rs".into(), CLI_LIB.into()),
        ("crates/caf-cli/src/bin/caf-cli.rs".into(), CLI_BIN.into()),
    ]
}

const APP_TOML: &str = r#"[product]
# CA brand standard — generated by `cashell product new`
bundle_id = "com.cecepazhar.__SLUG__"
name = "__SLUG__"
version = "0.1.0"
template = "__TEMPLATE__"
accent = "red-rose-amber"
standard = "CA"
"#;

const CAFRAMEWORK_YAML: &str = r#"# CAFramework bindings — generated by `cashell product new`
runner: cargo
channel: local
channels:
  local:
    runner: cargo
    target: native
  release:
    runner: cargo
    target: x86_64-unknown-linux-musl
"#;

const WORKSPACE_CARGO: &str = r#"[workspace]
resolver = "2"
members = [
    "crates/caf-app",
    "crates/caf-core",
    "crates/caf-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
caf-core = { path = "crates/caf-core" }

[profile.release]
opt-level = 3
lto = true
strip = true
codegen-units = 1
"#;

const GITIGNORE: &str = r#"/target
**/*.rs.bk
node_modules/
dist/
build/
.env
*.swp
.DS_Store
.idea/
.vscode/
"#;

const RUSTFMT_TOML: &str = r#"edition = "2021"
max_width = 100
"#;

const CLIPPY_TOML: &str = r#"avoid-breaking-exported-api = false
"#;

const README: &str = r#"# __SLUG__

CA Standard product (`__TEMPLATE__` template) — generated by `cashell product new`.

## Layout

- `crates/caf-core` — shared core library
- `crates/caf-app` — application binary entry
- `crates/caf-cli` — command line interface

## Build

```sh
cargo build
```
"#;

const CORE_CARGO: &str = r#"[package]
name = "caf-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
"#;

const CORE_LIB: &str = r#"//! caf-core — shared core library.

use std::fmt;

/// Core library version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Core error type.
#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

/// Returns the core library version.
pub fn version() -> &'static str {
    VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }
}
"#;

const APP_CARGO: &str = r#"[package]
name = "caf-app"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
caf-core.workspace = true
"#;

const APP_MAIN: &str = r#"//! caf-app — application entry point.

fn main() {
    println!("{} {}", env!("CARGO_PKG_NAME"), caf_core::version());
}
"#;

const CLI_CARGO: &str = r#"[package]
name = "caf-cli"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
caf-core.workspace = true
"#;

const CLI_LIB: &str = r#"//! caf-cli — command line interface library.

/// Greeting line for the CLI.
pub fn greet() -> String {
    format!("caf-cli {}", caf_core::version())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_contains_name() {
        assert!(greet().contains("caf-cli"));
    }
}
"#;

const CLI_BIN: &str = r#"//! caf-cli binary entry.

fn main() {
    println!("{}", caf_cli::greet());
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_good_slugs() {
        for s in ["caterm", "fath-store", "ca-2", "a"] {
            assert!(validate_slug(s).is_ok(), "should accept {s}");
        }
    }

    #[test]
    fn rejects_bad_slugs() {
        for s in [
            "",
            "Caterm",
            "-lead",
            "trail-",
            "has space",
            "under_score",
            "dot.slug",
        ] {
            assert!(validate_slug(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn parses_templates() {
        assert_eq!(Template::parse("desktop").unwrap(), Template::Desktop);
        assert_eq!(Template::parse("cli").unwrap(), Template::Cli);
        assert_eq!(Template::parse("service").unwrap(), Template::Service);
        assert!(Template::parse("web").is_err());
    }
}
