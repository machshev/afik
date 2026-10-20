//! The Maidenhead locators the operator keeps, each with an optional
//! description such as `Home`, for the generators' shared locator.
//!
//! They are kept in `$XDG_STATE_HOME/afik/locators`, one per line: the
//! locator, then the description.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use crate::files as fs;

use crate::{paths, repeaters::locator_centre};

const FILE: &str = "locators";

/// One kept locator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedLocator {
    /// The locator, upper case.
    pub locator: String,
    /// What the operator calls it, such as `Home`; may be empty.
    pub description: String,
}

impl SavedLocator {
    /// How the dropdown shows it: `Home (IO91WM)`, or the locator alone.
    pub fn label(&self) -> String {
        if self.description.is_empty() {
            self.locator.clone()
        } else {
            format!("{} ({})", self.description, self.locator)
        }
    }
}

/// Every kept locator, in the order they were first saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedLocators {
    /// The locators.
    pub list: Vec<SavedLocator>,
}

impl SavedLocators {
    /// The state file the locators are kept in.
    pub fn file() -> Option<PathBuf> {
        paths::state_dir().map(|dir| dir.join(FILE))
    }

    /// Reads the file, skipping lines that are not a locator; with no file,
    /// none.
    pub fn load(file: &Path) -> Self {
        let text = fs::read_to_string(file).unwrap_or_default();
        let mut locators = Self::default();
        for line in text.lines() {
            let (locator, description) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
            // A bad line is dropped, not the whole file.
            let _ = locators.put(locator, description);
        }
        locators
    }

    /// Writes the locators for the next start.
    pub fn save(&self, file: &Path) -> Result<(), String> {
        let text = self.list.iter().fold(String::new(), |mut text, saved| {
            let _ = writeln!(text, "{} {}", saved.locator, saved.description);
            text
        });
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, text))
            .map_err(|error| format!("{}: {error}", file.display()))
    }

    /// The kept locator matching `locator`, whatever its case.
    pub fn find(&self, locator: &str) -> Option<&SavedLocator> {
        let locator = locator.trim();
        self.list
            .iter()
            .find(|saved| saved.locator.eq_ignore_ascii_case(locator))
    }

    /// Keeps `locator` with `description`, replacing the description of one
    /// already kept; refuses anything that is not a locator.
    pub fn put(&mut self, locator: &str, description: &str) -> Result<(), String> {
        locator_centre(locator)?;
        let locator = locator.trim().to_ascii_uppercase();
        let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
        match self.list.iter_mut().find(|saved| saved.locator == locator) {
            Some(saved) => saved.description = description,
            None => self.list.push(SavedLocator {
                locator,
                description,
            }),
        }
        Ok(())
    }

    /// Forgets `locator`; whether it was kept.
    pub fn remove(&mut self, locator: &str) -> bool {
        let before = self.list.len();
        let locator = locator.trim();
        self.list
            .retain(|saved| !saved.locator.eq_ignore_ascii_case(locator));
        self.list.len() < before
    }
}

#[cfg(test)]
mod tests {
    use super::SavedLocators;

    #[test]
    fn locators_are_kept_once_each_with_their_description() {
        let mut locators = SavedLocators::default();
        locators.put("io91wm", "  Home  sweet ").unwrap();
        locators.put("JN18EU", "").unwrap();
        assert!(locators.put("nonsense", "Work").is_err());
        locators.put("IO91WM", "Home").unwrap();
        let labels: Vec<_> = locators
            .list
            .iter()
            .map(super::SavedLocator::label)
            .collect();
        assert_eq!(labels, ["Home (IO91WM)", "JN18EU"]);
        assert_eq!(locators.find(" jn18eu").unwrap().locator, "JN18EU");

        let dir = std::env::temp_dir().join(format!("afik-locators-{}", std::process::id()));
        let file = dir.join("locators");
        assert_eq!(SavedLocators::load(&file), SavedLocators::default());
        locators.save(&file).unwrap();
        assert_eq!(SavedLocators::load(&file), locators);
        std::fs::write(&file, "IO91WM Home\nnot a locator\n\nJN18EU\n").unwrap();
        assert_eq!(SavedLocators::load(&file), locators, "bad lines skipped");

        assert!(locators.remove("io91wm"));
        assert!(!locators.remove("IO91WM"));
        assert_eq!(locators.list.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
