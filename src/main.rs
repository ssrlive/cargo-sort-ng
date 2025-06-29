use std::{fmt::Display, fs::read_to_string, io::Write, path::PathBuf};

use clap::{crate_authors, crate_name, crate_version};
use fmt::Config;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use toml_edit::{DocumentMut, Item};

mod fmt;
mod sort;
#[cfg(test)]
mod test_utils;

const CARGO_TOML: &str = "Cargo.toml";

const EXTRA_HELP: &str = r#"
NOTE: formatting is applied after the check for sorting so sorted but unformatted toml will not cause a failure.
"#;

type Result<T, E = Box<dyn std::error::Error + Send + Sync + 'static>> = std::result::Result<T, E>;

#[macro_export]
macro_rules! version_0 {
    () => {
        concat!("v", crate_version!())
    };
}

#[macro_export]
macro_rules! version_info {
    () => {
        concat!(crate_name!(), " ", $crate::version_0!())
    };
}

fn about_info() -> String {
    format!(
        "{}\n{}\n{}",
        version_info!(),
        crate_authors!(", "),
        "Ensure Cargo.toml dependency tables are sorted.",
    )
}

fn cargo_subcommand() -> String {
    let name = crate_name!();
    const PRIFIX: &str = "cargo-";
    if let Some(tail) = name.strip_prefix(PRIFIX) {
        format!("cargo {tail}")
    } else {
        name.to_owned()
    }
}

#[derive(clap::Parser, Debug)]
#[command(author = crate_authors!(", "), version = version_0!(), about = about_info(), bin_name = cargo_subcommand(), after_help = EXTRA_HELP)]
pub struct Cli {
    /// sets cwd, must contain a Cargo.toml file
    #[arg(value_name = "CWD")]
    pub cwd: Vec<String>,

    /// Returns non-zero exit code if Cargo.toml is unsorted, overrides default behavior
    #[arg(short, long)]
    pub check: bool,

    /// Prints Cargo.toml, lexically sorted, to stdout
    #[arg(short, long, conflicts_with = "check")]
    pub print: bool,

    /// Skips formatting after sorting
    #[arg(short = 'n', long)]
    pub no_format: bool,

    /// Also returns non-zero exit code if formatting changes
    #[arg(long, requires = "check")]
    pub check_format: bool,

    /// Checks every crate in a workspace
    #[arg(short, long)]
    pub workspace: bool,

    /// Keep blank lines when sorting groups of key value pairs
    #[arg(short, long)]
    pub grouped: bool,

    /// List the order tables should be written out
    /// (--order package,dependencies,features)
    #[arg(short, long, value_delimiter = ',')]
    pub order: Vec<String>,
}

fn write_red<S: Display>(highlight: &str, msg: S) -> Result<()> {
    let mut stderr = StandardStream::stderr(ColorChoice::Auto);
    stderr.set_color(ColorSpec::new().set_fg(Some(Color::Red)))?;
    write!(stderr, "{highlight}")?;
    stderr.reset()?;
    writeln!(stderr, "{msg}").map_err(Into::into)
}

fn write_green<S: Display>(highlight: &str, msg: S) -> Result<()> {
    let mut stdout = StandardStream::stdout(ColorChoice::Auto);
    stdout.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
    write!(stdout, "{highlight}")?;
    stdout.reset()?;
    writeln!(stdout, "{msg}").map_err(Into::into)
}

fn check_toml(path: &str, cli: &Cli, config: &Config) -> Result<bool> {
    let mut path = PathBuf::from(path);
    if path.is_dir() {
        path.push(CARGO_TOML);
    }

    let krate = path.components().nth_back(1).ok_or("No crate folder found")?.as_os_str();

    write_green("Checking ", format!("{}...", krate.to_string_lossy()))?;

    let toml_raw = read_to_string(&path).map_err(|_| format!("No file found at: {}", path.display()))?;

    let crlf = toml_raw.contains("\r\n");

    let mut config = config.clone();
    if config.crlf.is_none() {
        config.crlf = Some(crlf);
    }

    let mut sorted_doc = sort::sort_toml(&toml_raw, sort::MATCHER, cli.grouped, &config.table_order);

    // if no-format is not found apply formatting
    let (origin_already_formatted, mut final_str) = if !cli.no_format || cli.check_format {
        let before_fmt = sorted_doc.to_string();
        fmt::fmt_toml(&mut sorted_doc, &config);
        let final_str = sorted_doc.to_string();
        (before_fmt == final_str, final_str)
    } else {
        (true, sorted_doc.to_string())
    };

    if config.crlf.unwrap_or(fmt::DEF_CRLF) && !final_str.contains("\r\n") {
        final_str = final_str.replace('\n', "\r\n");
    }

    if cli.print {
        print!("{final_str}");
        return Ok(true);
    }

    let origin_already_sorted = toml_raw == final_str;
    if cli.check {
        if !origin_already_sorted {
            write_red("error: ", format!("Dependencies for {} are not sorted", krate.to_string_lossy()))?;
        }

        if !origin_already_formatted {
            write_red("error: ", format!("{CARGO_TOML} for {} is not formatted", krate.to_string_lossy()))?;
        }

        return Ok(origin_already_sorted && origin_already_formatted);
    }

    if !origin_already_sorted {
        std::fs::write(&path, &final_str)?;
        let msg = format!("{CARGO_TOML} for {:?} has been rewritten", krate.to_string_lossy());
        write_green("Finished: ", msg)?;
    } else {
        let msg = format!("{CARGO_TOML} for {} is sorted already, no changes made", krate.to_string_lossy());
        write_green("Finished: ", msg)?;
    }

    Ok(true)
}

fn _main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    // remove "sort-fix" when invoked `cargo sort-fix` sort-fix is the first arg
    // https://github.com/rust-lang/cargo/issues/7653
    if args.len() > 1 && args[1] == "sort-fix" {
        args.remove(1);
    }
    let cli = <Cli as clap::Parser>::parse_from(args);

    let cwd = std::env::current_dir().map_err(|e| format!("no current directory found: {e}"))?;
    let dir = cwd.to_string_lossy();

    let mut filtered_matches: Vec<String> = cli.cwd.clone();
    let is_posible_workspace = filtered_matches.is_empty() || filtered_matches.len() == 1;
    if filtered_matches.is_empty() {
        filtered_matches.push(dir.to_string());
    }

    if cli.workspace && is_posible_workspace {
        let mut file_path = PathBuf::from(&&filtered_matches[0]);
        let dir = if file_path.is_file() {
            let mut path_dir = file_path.clone();
            path_dir.pop();
            path_dir.to_string_lossy().to_string()
        } else if file_path.is_dir() {
            let path_dir = file_path.clone();
            file_path.push(CARGO_TOML);
            path_dir.to_string_lossy().to_string()
        } else {
            let m = format!("Item `{}` is not a file or directory", file_path.display());
            return Err(m.into());
        };

        let raw_toml = read_to_string(&file_path).map_err(|_| format!("no file found at: {}", file_path.display()))?;

        let toml = raw_toml.parse::<DocumentMut>()?;
        let workspace = toml.get("workspace");
        if let Some(Item::Table(ws)) = workspace {
            // The workspace excludes, used to filter members by
            let excludes = workspace_items_of_kind(&dir, ws, "exclude")?;
            let members = workspace_items_of_kind(&dir, ws, "members")?;
            'globs: for member in &members {
                // The `check_toml` function expects only folders that it appends `Cargo.toml` onto
                if member.is_file() {
                    continue;
                }
                for excl in &excludes {
                    if member == excl {
                        continue 'globs;
                    }
                }
                filtered_matches.push(member.display().to_string());
            }
        }
    }

    let mut cwd = cwd.clone();
    cwd.push("tomlfmt.toml");
    let mut config = read_to_string(&cwd)
        .or_else(|_err| {
            cwd.pop();
            cwd.push(".tomlfmt.toml");
            read_to_string(&cwd)
        })
        .unwrap_or_default()
        .parse::<Config>()?;

    if !cli.order.is_empty() {
        config.table_order = cli.order.clone();
    }

    let mut flag = true;
    for sorted in filtered_matches.iter().map(|path| check_toml(path, &cli, &config)) {
        match sorted {
            Ok(true) => continue,
            Ok(false) => flag = false,
            Err(e) => {
                write_red("error: ", e)?;
                flag = false;
            }
        }
    }

    if !flag {
        return Err("Some Cargo.toml files are not sorted or formatted".into());
    }
    Ok(())
}

fn array_string_members(value: &Item) -> Vec<&str> {
    value.as_array().into_iter().flatten().filter_map(|s| s.as_str()).collect()
}

fn workspace_items_of_kind(dir: &str, ws: &toml_edit::Table, kind: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for member in ws.get(kind).map_or_else(Vec::new, array_string_members) {
        // TODO: a better test wether to glob?
        if member.contains('*') || member.contains('?') {
            let paths_iter = glob::glob(&format!("{dir}/{member}"))?;
            for path in paths_iter {
                paths.push(path?);
            }
        } else {
            let mut path = PathBuf::from(dir);
            path.push(member);
            paths.push(path);
        }
    }
    Ok(paths)
}

fn main() {
    _main().unwrap_or_else(|e| {
        write_red("error: ", e).unwrap();
        std::process::exit(1);
    });
}

// #[test]
// fn fuzzy_fail() {
//     for file in std::fs::read_dir("out/default/crashes").unwrap() {
//         let path = file.unwrap().path();
//         println!("{}", path.display());
//         let s = read_to_string(&path).unwrap().replace("\r", "");
//         let mut toml = sort::sort_toml(&s, sort::MATCHER, false);
//         fmt::fmt_toml(&mut toml, &fmt::Config::default());
//         print!("{}", s);
//         s.parse::<DocumentMut>().unwrap();
//     }
// }
