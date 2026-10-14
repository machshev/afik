//! Named projects kept under the XDG data directory.
//!
//! Each project is one file, `$XDG_DATA_HOME/afik/projects/NAME.afik`, so the
//! operator picks a project by name and never types a path. The last project
//! opened is remembered in `$XDG_STATE_HOME/afik/last-project` and opened
//! again at start-up.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::paths;

/// Extension of a project file.
pub const PROJECT_EXTENSION: &str = "afik";
/// Name of the project used when none exists yet.
pub const DEFAULT_PROJECT: &str = "default";
/// Longest project name accepted.
pub const MAX_NAME_LEN: usize = 64;

/// One project the operator can choose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEntry {
    /// Name shown in the project list.
    pub name: String,
    /// File the project is loaded from and saved to.
    pub path: PathBuf,
}

/// The projects the operator can choose from, and which one is open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectStore {
    dir: PathBuf,
    last: Option<PathBuf>,
    entries: Vec<ProjectEntry>,
    current: usize,
}

impl ProjectStore {
    /// Opens the store under the XDG directories.
    pub fn open() -> Self {
        Self::open_in(
            paths::projects_dir().unwrap_or_else(|| PathBuf::from("projects")),
            paths::last_project_file(),
        )
    }

    /// Opens the store in `dir`, remembering the last project in `last`.
    ///
    /// The last project opened is selected; failing that the first by name;
    /// failing that a project named [`DEFAULT_PROJECT`], which exists once it
    /// is first saved.
    pub fn open_in(dir: PathBuf, last: Option<PathBuf>) -> Self {
        let mut entries = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension == PROJECT_EXTENSION)
            })
            .filter_map(|path| {
                let name = path.file_stem()?.to_str()?.to_owned();
                Some(ProjectEntry { name, path })
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        if entries.is_empty() {
            entries.push(ProjectEntry {
                name: DEFAULT_PROJECT.to_owned(),
                path: project_path(&dir, DEFAULT_PROJECT),
            });
        }
        let remembered = last
            .as_deref()
            .and_then(|file| fs::read_to_string(file).ok())
            .map(|text| text.trim().to_owned());
        let current = remembered
            .and_then(|name| entries.iter().position(|entry| entry.name == name))
            .unwrap_or(0);
        Self {
            dir,
            last,
            entries,
            current,
        }
    }

    /// Every project, by name.
    pub fn entries(&self) -> &[ProjectEntry] {
        &self.entries
    }

    /// The open project.
    pub fn current(&self) -> &ProjectEntry {
        &self.entries[self.current]
    }

    /// Index of the open project in [`Self::entries`].
    pub const fn current_index(&self) -> usize {
        self.current
    }

    /// Opens project `index` and remembers it as the last one opened.
    pub fn select(&mut self, index: usize) {
        if index < self.entries.len() {
            self.current = index;
            self.remember();
        }
    }

    /// Adds a project named `name` and opens it.
    ///
    /// The caller saves the new project, which creates its file.
    pub fn create(&mut self, name: &str) -> Result<&ProjectEntry, String> {
        let name = valid_name(name)?;
        if self.entries.iter().any(|entry| entry.name == name) {
            return Err(format!("A project named {name} already exists."));
        }
        let path = project_path(&self.dir, &name);
        if path.exists() {
            return Err(format!("{} already exists.", path.display()));
        }
        let at = self
            .entries
            .iter()
            .position(|entry| entry.name > name)
            .unwrap_or(self.entries.len());
        self.entries.insert(at, ProjectEntry { name, path });
        self.select(at);
        Ok(self.current())
    }

    /// Adds a project file named on the command line, wherever it is, and
    /// opens it.
    pub fn adopt(&mut self, path: &Path) {
        if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
            self.select(index);
            return;
        }
        let name = path.file_stem().map_or_else(
            || path.display().to_string(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        self.entries.push(ProjectEntry {
            name,
            path: path.to_owned(),
        });
        self.select(self.entries.len() - 1);
    }

    /// Records the open project so the next start opens it again.
    fn remember(&self) {
        let Some(file) = &self.last else {
            return;
        };
        let result = file
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, format!("{}\n", self.current().name)));
        if let Err(error) = result {
            log::warn!(
                "could not record the last project in {}: {error}",
                file.display()
            );
        }
    }
}

/// Returns the file a project named `name` is kept in.
pub fn project_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{PROJECT_EXTENSION}"))
}

/// Checks a new project name: one file name, readable in a list.
pub fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name for the new project.".to_owned());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "A project name is at most {MAX_NAME_LEN} characters."
        ));
    }
    if name.starts_with('.')
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        return Err(
            "A project name uses letters, digits, spaces, '-', '_' and '.', \
             and does not start with '.'."
                .to_owned(),
        );
    }
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{valid_name, ProjectStore, DEFAULT_PROJECT};
    use std::{fs, path::PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("afik-projects-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_empty_store_offers_the_default_project() {
        let dir = scratch("empty");
        let store = ProjectStore::open_in(dir.join("projects"), None);
        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.current().name, DEFAULT_PROJECT);
        assert_eq!(
            store.current().path,
            dir.join("projects").join("default.afik")
        );
    }

    #[test]
    fn the_last_project_opened_is_opened_again() {
        let dir = scratch("last");
        let projects = dir.join("projects");
        let last = dir.join("state").join("last-project");
        fs::create_dir_all(&projects).unwrap();
        for name in ["beta", "alpha", "notes"] {
            fs::write(projects.join(format!("{name}.afik")), [0_u8]).unwrap();
        }
        fs::write(projects.join("readme.txt"), [0_u8]).unwrap();

        let mut store = ProjectStore::open_in(projects.clone(), Some(last.clone()));
        let names: Vec<_> = store.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "notes"]);
        assert_eq!(store.current().name, "alpha");

        store.select(1);
        assert_eq!(fs::read_to_string(&last).unwrap(), "beta\n");
        let store = ProjectStore::open_in(projects.clone(), Some(last.clone()));
        assert_eq!(store.current().name, "beta");

        // A remembered project which has gone falls back to the first.
        fs::remove_file(projects.join("beta.afik")).unwrap();
        let store = ProjectStore::open_in(projects, Some(last));
        assert_eq!(store.current().name, "alpha");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_project_is_listed_in_order_and_opened() {
        let dir = scratch("create");
        let last = dir.join("last-project");
        let mut store = ProjectStore::open_in(dir.join("projects"), Some(last.clone()));
        let created = store.create("  Hill walk ").unwrap().clone();
        assert_eq!(created.name, "Hill walk");
        assert_eq!(created.path, dir.join("projects").join("Hill walk.afik"));
        let names: Vec<_> = store.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Hill walk", "default"]);
        assert_eq!(store.current_index(), 0);
        assert_eq!(fs::read_to_string(&last).unwrap(), "Hill walk\n");
        assert!(store.create("Hill walk").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_project_file_named_on_the_command_line_is_listed_and_opened() {
        let dir = scratch("adopt");
        let mut store = ProjectStore::open_in(dir.join("projects"), None);
        store.adopt(&dir.join("elsewhere").join("club.afik"));
        assert_eq!(store.current().name, "club");
        assert_eq!(store.entries().len(), 2);
        store.adopt(&dir.join("elsewhere").join("club.afik"));
        assert_eq!(store.entries().len(), 2);
    }

    #[test]
    fn project_names_are_one_readable_file_name() {
        assert_eq!(valid_name(" PMR "), Ok("PMR".to_owned()));
        assert_eq!(valid_name("club_2m-v1.2"), Ok("club_2m-v1.2".to_owned()));
        for bad in [
            "",
            "   ",
            "a/b",
            "..",
            ".hidden",
            "tab\there",
            &"x".repeat(65),
        ] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }
}
