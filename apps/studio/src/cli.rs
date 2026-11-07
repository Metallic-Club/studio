//! The command line: open Studio, open a `.mtproj` straight into the editor, or register Studio for `.mtproj` files
//! (ADR 0009, Opening `.mtproj` files).

use std::ffi::OsString;
use std::path::PathBuf;

/// `metallic_studio --help`.
pub const HELP: &str = "\
Metallic Studio: creates and edits Metallic projects.

Usage: metallic_studio [OPTIONS] [PROJECT.mtproj]

Arguments:
  [PROJECT.mtproj]           A project to open straight into the editor (it is added to Recent)

Options:
  --register-file-types      Open .mtproj files with this Studio for the current user, print what was changed, exit
  --unregister-file-types    Undo --register-file-types, print what was removed, exit
  -h, --help                 Print this help
  -V, --version              Print the version";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Open Studio, and the project at this path (made absolute) if there is one.
    Run {
        /// The project to open straight into the editor.
        open: Option<PathBuf>,
    },
    /// Associate `.mtproj` files with this executable for the current user.
    RegisterFileTypes,
    /// Remove that association.
    UnregisterFileTypes,
    /// Print the help.
    Help,
    /// Print the version.
    Version,
}

impl Command {
    /// Parses the arguments after the program name. A relative project path is made absolute against the working
    /// folder now, so it means the same thing whatever Studio does later.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, lexopt::Error> {
        use lexopt::prelude::*;

        let mut command = None;
        let mut open = None;
        let mut parser = lexopt::Parser::from_args(args);
        let mut set = |next: Command| match command.replace(next) {
            Some(_) => Err(lexopt::Error::from(
                "give one of --register-file-types and --unregister-file-types",
            )),
            None => Ok(()),
        };
        while let Some(arg) = parser.next()? {
            match arg {
                Long("register-file-types") => set(Command::RegisterFileTypes)?,
                Long("unregister-file-types") => set(Command::UnregisterFileTypes)?,
                Short('h') | Long("help") => return Ok(Command::Help),
                Short('V') | Long("version") => return Ok(Command::Version),
                Value(path) if open.is_none() => open = Some(PathBuf::from(path)),
                _ => return Err(arg.unexpected()),
            }
        }
        match (command, open) {
            (Some(_), Some(_)) => Err("a project to open and --(un)register-file-types do not go together".into()),
            (Some(command), None) => Ok(command),
            (None, open) => Ok(Command::Run {
                // `absolute` fails only on an empty path, which then fails to open and says so.
                open: open.map(|path| std::path::absolute(&path).unwrap_or(path)),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, lexopt::Error> {
        Command::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_just_opens_studio() {
        assert_eq!(parse(&[]).unwrap(), Command::Run { open: None });
    }

    #[test]
    fn a_project_path_is_made_absolute_and_keeps_its_spaces() {
        let Command::Run { open: Some(path) } = parse(&["My Games/Space Race.mtproj"]).unwrap() else {
            panic!("expected a project to open");
        };
        assert!(path.is_absolute(), "{}", path.display());
        assert_eq!(
            path,
            std::env::current_dir()
                .unwrap()
                .join("My Games")
                .join("Space Race.mtproj")
        );

        let absolute = std::env::temp_dir().join("with spaces").join("A Game.mtproj");
        let given = absolute.to_str().unwrap();
        assert_eq!(parse(&[given]).unwrap(), Command::Run { open: Some(absolute) });
    }

    #[test]
    fn the_file_type_commands_stand_alone() {
        assert_eq!(parse(&["--register-file-types"]).unwrap(), Command::RegisterFileTypes);
        assert_eq!(
            parse(&["--unregister-file-types"]).unwrap(),
            Command::UnregisterFileTypes
        );
        assert_eq!(parse(&["--help"]).unwrap(), Command::Help);
        assert_eq!(parse(&["-V"]).unwrap(), Command::Version);
        assert!(parse(&["--register-file-types", "--unregister-file-types"]).is_err());
        assert!(parse(&["--register-file-types", "a.mtproj"]).is_err());
        assert!(parse(&["a.mtproj", "b.mtproj"]).is_err());
        assert!(parse(&["--frobnicate"]).is_err());
    }
}
