//! Operator confirmation for commands which write to a radio.
//!
//! A write used to be gated by typed phrases the operator had to know in
//! advance. Those proved nothing about the run in front of them: a phrase can be
//! pasted from a note without ever reading what it is about to do. What actually
//! makes a write deliberate is showing the operator the exact device, image, and
//! digest and having them agree, which is what this module is for.
//!
//! It is a trait so the decision is testable without a terminal, and so a
//! non-interactive run fails closed rather than silently assuming yes.

use std::io::{self, BufRead, IsTerminal, Write};

/// How a command asks the operator to decide something.
pub trait Confirm {
    /// Asks the operator to approve an action described by `summary`.
    ///
    /// Returns `false` for anything but an explicit yes, so a stray newline or a
    /// closed input declines rather than proceeds.
    fn confirm(&mut self, summary: &str) -> io::Result<bool>;

    /// Asks the operator to pick one of several options.
    ///
    /// Returns `None` when nobody can answer, which leaves the caller to report
    /// the ambiguity rather than guess at it.
    fn choose(&mut self, summary: &str, options: &[String]) -> io::Result<Option<usize>>;

    /// Reports whether this confirmer can actually ask a question.
    fn is_interactive(&self) -> bool;
}

/// Asks on the terminal the command was started from.
pub struct TerminalConfirm<R, W> {
    input: R,
    output: W,
    interactive: bool,
}

impl<R: BufRead, W: Write> TerminalConfirm<R, W> {
    /// Builds a confirmer over explicit streams.
    ///
    /// `interactive` is supplied rather than detected so a test can drive both
    /// paths, and so the caller decides what counts as a terminal.
    pub const fn new(input: R, output: W, interactive: bool) -> Self {
        Self {
            input,
            output,
            interactive,
        }
    }

    fn read_line(&mut self) -> io::Result<String> {
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            // Closed input is not agreement.
            return Ok(String::new());
        }
        Ok(line.trim().to_owned())
    }
}

impl<R: BufRead, W: Write> Confirm for TerminalConfirm<R, W> {
    fn confirm(&mut self, summary: &str) -> io::Result<bool> {
        if !self.interactive {
            return Ok(false);
        }
        write!(self.output, "{summary}\nProceed? [y/N] ")?;
        self.output.flush()?;
        let answer = self.read_line()?;
        Ok(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
    }

    fn choose(&mut self, summary: &str, options: &[String]) -> io::Result<Option<usize>> {
        if !self.interactive || options.is_empty() {
            return Ok(None);
        }
        writeln!(self.output, "{summary}")?;
        for (index, option) in options.iter().enumerate() {
            writeln!(self.output, "  {}) {option}", index + 1)?;
        }
        write!(self.output, "Choose 1-{}: ", options.len())?;
        self.output.flush()?;
        let answer = self.read_line()?;
        let Ok(choice) = answer.parse::<usize>() else {
            return Ok(None);
        };
        if choice == 0 || choice > options.len() {
            return Ok(None);
        }
        Ok(Some(choice - 1))
    }

    fn is_interactive(&self) -> bool {
        self.interactive
    }
}

/// Approves everything without asking.
///
/// This exists for `--yes`, where the operator has already decided, and for a
/// script which cannot answer a prompt. It is never the default: a run which
/// neither asked nor was told to assume yes must stop.
pub struct AssumeYes;

impl Confirm for AssumeYes {
    fn confirm(&mut self, _summary: &str) -> io::Result<bool> {
        Ok(true)
    }

    fn choose(&mut self, _summary: &str, _options: &[String]) -> io::Result<Option<usize>> {
        // An unattended run must not pick a radio on the operator's behalf.
        Ok(None)
    }

    fn is_interactive(&self) -> bool {
        false
    }
}

/// Builds the confirmer for a real run.
#[must_use]
pub fn terminal() -> TerminalConfirm<io::BufReader<io::Stdin>, io::Stderr> {
    let interactive = io::stdin().is_terminal() && io::stderr().is_terminal();
    // Questions go to stderr so a redirected stdout still captures only the
    // machine-readable run log.
    TerminalConfirm::new(io::BufReader::new(io::stdin()), io::stderr(), interactive)
}

#[cfg(test)]
mod tests {
    use super::{AssumeYes, Confirm, TerminalConfirm};

    fn confirmer(answer: &str) -> TerminalConfirm<&[u8], Vec<u8>> {
        TerminalConfirm::new(answer.as_bytes(), Vec::new(), true)
    }

    #[test]
    fn only_an_explicit_yes_approves_a_write() {
        assert!(confirmer("y\n").confirm("write").unwrap());
        assert!(confirmer("Y\n").confirm("write").unwrap());
        assert!(confirmer("yes\n").confirm("write").unwrap());
        assert!(!confirmer("\n").confirm("write").unwrap());
        assert!(!confirmer("n\n").confirm("write").unwrap());
        assert!(!confirmer("maybe\n").confirm("write").unwrap());
        assert!(
            !confirmer("").confirm("write").unwrap(),
            "closed input is not agreement"
        );
    }

    #[test]
    fn a_run_which_cannot_ask_declines_rather_than_proceeds() {
        let mut silent = TerminalConfirm::new(&b"y\n"[..], Vec::new(), false);
        assert!(
            !silent.confirm("write").unwrap(),
            "a non-interactive run must not be approved by whatever is on stdin"
        );
        assert_eq!(silent.choose("pick", &["a".to_owned()]).unwrap(), None);
    }

    #[test]
    fn a_choice_is_one_based_and_bounded() {
        let options = ["/dev/ttyUSB0".to_owned(), "/dev/ttyUSB1".to_owned()];
        assert_eq!(confirmer("1\n").choose("pick", &options).unwrap(), Some(0));
        assert_eq!(confirmer("2\n").choose("pick", &options).unwrap(), Some(1));
        assert_eq!(confirmer("0\n").choose("pick", &options).unwrap(), None);
        assert_eq!(confirmer("3\n").choose("pick", &options).unwrap(), None);
        assert_eq!(confirmer("x\n").choose("pick", &options).unwrap(), None);
        assert_eq!(confirmer("\n").choose("pick", &options).unwrap(), None);
    }

    #[test]
    fn assumed_yes_approves_but_still_will_not_pick_a_radio() {
        assert!(AssumeYes.confirm("write").unwrap());
        assert_eq!(
            AssumeYes
                .choose("pick", &["a".to_owned(), "b".to_owned()])
                .unwrap(),
            None,
            "an unattended run must not choose which radio to write to"
        );
    }
}
