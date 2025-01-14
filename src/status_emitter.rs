//! Variaous schemes for reporting messages during testing or after testing is done.

use bstr::ByteSlice;
use color_eyre::eyre::bail;
use colored::Colorize;

use crate::{
    github_actions, parser::Pattern, rustc_stderr::Message, Config, Error, Result, TestResult,
};
use std::{
    fmt::{Debug, Write as _},
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

/// A generic way to handle the output of this crate.
pub trait StatusEmitter: Sync {
    /// Invoked before each failed test prints its errors along with a drop guard that can
    /// gets invoked afterwards.
    fn failed_test<'a>(
        &'a self,
        revision: &'a str,
        path: &'a Path,
        cmd: &'a Command,
        stderr: &'a [u8],
    ) -> Box<dyn Debug + 'a>;

    /// Start a test run and return a handle for reporting individual tests' results
    fn run_tests(&self, config: &Config) -> Box<dyn DuringTestRun>;
    /// Create a report about the entire test run at the end.
    #[allow(clippy::type_complexity)]
    fn finalize(
        &self,
        _failures: &[(PathBuf, Command, String, Vec<Error>, Vec<u8>)],
        _succeeded: usize,
        _ignored: usize,
        _filtered: usize,
    ) -> Result<()> {
        Ok(())
    }
}

/// Report information during test runs.
pub trait DuringTestRun {
    /// A test has finished, handle the result.
    fn test_result(&mut self, _path: &Path, _revision: &str, _result: &TestResult) {}
}

/// A human readable output emitter.
pub struct Text;
impl StatusEmitter for Text {
    fn failed_test<'a>(
        &self,
        revision: &str,
        path: &Path,
        cmd: &Command,
        stderr: &'a [u8],
    ) -> Box<dyn Debug + 'a> {
        eprintln!();
        let path = path.display().to_string();
        eprint!("{}", path.underline().bold());
        let revision = if revision.is_empty() {
            String::new()
        } else {
            format!(" (revision `{revision}`)")
        };
        eprint!("{revision}");
        eprint!(" {}", "FAILED:".red().bold());
        eprintln!();
        eprintln!("command: {cmd:?}");
        eprintln!();

        #[derive(Debug)]
        struct Guard<'a>(&'a [u8]);
        impl<'a> Drop for Guard<'a> {
            fn drop(&mut self) {
                eprintln!("full stderr:");
                std::io::stderr().write_all(self.0).unwrap();
                eprintln!();
                eprintln!();
            }
        }
        Box::new(Guard(stderr))
    }

    fn run_tests(&self, config: &Config) -> Box<dyn DuringTestRun> {
        if config.quiet {
            Box::new(Quiet { n: 0 })
        } else {
            Box::new(Text)
        }
    }

    fn finalize(
        &self,
        failures: &[(PathBuf, Command, String, Vec<Error>, Vec<u8>)],
        succeeded: usize,
        ignored: usize,
        filtered: usize,
    ) -> Result<()> {
        // Print all errors in a single thread to show reliable output
        if !failures.is_empty() {
            for (path, cmd, revision, errors, stderr) in failures {
                let _guard = self.failed_test(revision, path, cmd, stderr);
                for error in errors {
                    print_error(error, &path.display().to_string());
                }
            }
            eprintln!("{}", "FAILURES:".red().underline().bold());
            for (path, _cmd, _revision, _errors, _stderr) in failures {
                eprintln!("    {}", path.display());
            }
            eprintln!();
            eprintln!(
                "test result: {}. {} tests failed, {} tests passed, {} ignored, {} filtered out",
                "FAIL".red(),
                failures.len().to_string().red().bold(),
                succeeded.to_string().green(),
                ignored.to_string().yellow(),
                filtered.to_string().yellow(),
            );
            bail!("tests failed");
        }
        eprintln!();
        eprintln!(
            "test result: {}. {} tests passed, {} ignored, {} filtered out",
            "ok".green(),
            succeeded.to_string().green(),
            ignored.to_string().yellow(),
            filtered.to_string().yellow(),
        );
        eprintln!();
        Ok(())
    }
}

fn print_error(error: &Error, path: &str) {
    match error {
        Error::ExitStatus {
            mode,
            status,
            expected,
        } => {
            eprintln!("{mode} test got {status}, but expected {expected}")
        }
        Error::Command { kind, status } => {
            eprintln!("{kind} failed with {status}");
        }
        Error::PatternNotFound {
            pattern,
            definition_line,
        } => {
            match pattern {
                Pattern::SubString(s) => {
                    eprintln!("substring `{s}` {} in stderr output", "not found".red())
                }
                Pattern::Regex(r) => {
                    eprintln!("`/{r}/` does {} stderr output", "not match".red())
                }
            }
            eprintln!(
                "expected because of pattern here: {}",
                format!("{path}:{definition_line}").bold()
            );
        }
        Error::NoPatternsFound => {
            eprintln!("{}", "no error patterns found in fail test".red());
        }
        Error::PatternFoundInPassTest => {
            eprintln!("{}", "error pattern found in pass test".red())
        }
        Error::OutputDiffers {
            path: output_path,
            actual,
            expected,
        } => {
            eprintln!("{}", "actual output differed from expected".underline());
            eprintln!("{}", format!("--- {}", output_path.display()).red());
            eprintln!("{}", "+++ <stderr output>".green());
            crate::diff::print_diff(expected, actual);
        }
        Error::ErrorsWithoutPattern { path: None, msgs } => {
            eprintln!(
                "There were {} unmatched diagnostics that occurred outside the testfile and had no pattern",
                msgs.len(),
            );
            for Message { level, message } in msgs {
                eprintln!("    {level:?}: {message}")
            }
        }
        Error::ErrorsWithoutPattern {
            path: Some((path, line)),
            msgs,
        } => {
            let path = path.display();
            eprintln!(
                "There were {} unmatched diagnostics at {path}:{line}",
                msgs.len(),
            );
            for Message { level, message } in msgs {
                eprintln!("    {level:?}: {message}")
            }
        }
        Error::InvalidComment { msg, line } => {
            let mut err =
                github_actions::error(path, format!("Could not parse comment")).line(*line);
            writeln!(err, "{msg}").unwrap();
            eprintln!("Could not parse comment in {path}:{line} because\n{msg}",)
        }
        Error::Bug(msg) => {
            eprintln!("A bug in `ui_test` occurred: {msg}");
        }
        Error::Aux {
            path: aux_path,
            errors,
            line,
        } => {
            eprintln!("Aux build from {path}:{line} failed");
            for error in errors {
                print_error(error, &aux_path.display().to_string());
            }
        }
    }
    eprintln!();
}

fn gha_error(error: &Error, path: &str, revision: &str) {
    match error {
        Error::ExitStatus {
            mode,
            status,
            expected,
        } => {
            github_actions::error(
                path,
                format!("{mode} test{revision} got {status}, but expected {expected}"),
            );
        }
        Error::Command { kind, status } => {
            github_actions::error(path, format!("{kind}{revision} failed with {status}"));
        }
        Error::PatternNotFound {
            pattern: _,
            definition_line,
        } => {
            github_actions::error(path, format!("Pattern not found{revision}"))
                .line(*definition_line);
        }
        Error::NoPatternsFound => {
            github_actions::error(
                path,
                format!("no error patterns found in fail test{revision}"),
            );
        }
        Error::PatternFoundInPassTest => {
            github_actions::error(path, format!("error pattern found in pass test{revision}"));
        }
        Error::OutputDiffers {
            path: output_path,
            actual,
            expected,
        } => {
            let mut err = github_actions::error(
                if expected.is_empty() {
                    path.to_owned()
                } else {
                    output_path.display().to_string()
                },
                "actual output differs from expected",
            );
            writeln!(err, "```diff").unwrap();
            let mut seen_diff_line = Some(0);
            for r in ::diff::lines(expected.to_str().unwrap(), actual.to_str().unwrap()) {
                if let Some(line) = &mut seen_diff_line {
                    *line += 1;
                }
                let mut seen_diff = || {
                    if let Some(line) = seen_diff_line.take() {
                        writeln!(err, "{line} unchanged lines skipped").unwrap();
                    }
                };
                match r {
                    ::diff::Result::Both(l, r) => {
                        if l != r {
                            seen_diff();
                            writeln!(err, "-{l}").unwrap();
                            writeln!(err, "+{r}").unwrap();
                        } else if seen_diff_line.is_none() {
                            writeln!(err, " {l}").unwrap()
                        }
                    }
                    ::diff::Result::Left(l) => {
                        seen_diff();
                        writeln!(err, "-{l}").unwrap();
                    }
                    ::diff::Result::Right(r) => {
                        seen_diff();
                        writeln!(err, "+{r}").unwrap();
                    }
                }
            }
            writeln!(err, "```").unwrap();
        }
        Error::ErrorsWithoutPattern { path: None, msgs } => {
            let mut err = github_actions::error(
                path,
                format!("Unmatched diagnostics outside the testfile{revision}"),
            );
            for Message { level, message } in msgs {
                writeln!(err, "{level:?}: {message}").unwrap();
            }
        }
        Error::ErrorsWithoutPattern {
            path: Some((path, line)),
            msgs,
        } => {
            let path = path.display();
            let mut err = github_actions::error(&path, format!("Unmatched diagnostics{revision}"))
                .line(*line);
            for Message { level, message } in msgs {
                writeln!(err, "{level:?}: {message}").unwrap();
            }
        }
        Error::InvalidComment { msg, line } => {
            let mut err =
                github_actions::error(path, format!("Could not parse comment")).line(*line);
            writeln!(err, "{msg}").unwrap();
        }
        Error::Bug(_) => {}
        Error::Aux {
            path: aux_path,
            errors,
            line,
        } => {
            github_actions::error(path, format!("Aux build failed")).line(*line);
            for error in errors {
                gha_error(error, &aux_path.display().to_string(), "")
            }
        }
    }
    eprintln!();
}

impl DuringTestRun for Text {
    fn test_result(&mut self, path: &Path, revision: &str, result: &TestResult) {
        let result = match result {
            TestResult::Ok => "ok".green(),
            TestResult::Errored { .. } => "FAILED".red().bold(),
            TestResult::Ignored => "ignored (in-test comment)".yellow(),
            TestResult::Filtered => return,
        };
        eprint!(
            "{}{} ... ",
            path.display(),
            if revision.is_empty() {
                "".into()
            } else {
                format!(" ({revision})")
            }
        );
        eprintln!("{result}");
    }
}

struct Quiet {
    n: usize,
}

impl DuringTestRun for Quiet {
    fn test_result(&mut self, _path: &Path, _revision: &str, result: &TestResult) {
        // Humans start counting at 1
        self.n += 1;
        match result {
            TestResult::Ok => eprint!("{}", ".".green()),
            TestResult::Errored { .. } => eprint!("{}", "F".red().bold()),
            TestResult::Ignored => eprint!("{}", "i".yellow()),
            TestResult::Filtered => {}
        }
        if self.n % 100 == 0 {
            eprintln!(" {}", self.n);
        }
    }
}

/// Emits Github Actions Workspace commands to show the failures directly in the github diff view.
pub struct Gha;
impl StatusEmitter for Gha {
    fn failed_test(
        &self,
        revision: &str,
        path: &Path,
        _cmd: &Command,
        _stderr: &[u8],
    ) -> Box<dyn Debug> {
        Box::new(github_actions::group(format_args!(
            "{}:{revision}",
            path.display()
        )))
    }

    fn run_tests(&self, _config: &Config) -> Box<dyn DuringTestRun> {
        Box::new(Gha)
    }

    fn finalize(
        &self,
        failures: &[(PathBuf, Command, String, Vec<Error>, Vec<u8>)],
        _succeeded: usize,
        _ignored: usize,
        _filtered: usize,
    ) -> Result<()> {
        for (path, cmd, revision, errors, stderr) in failures {
            let revision = if revision.is_empty() {
                "".to_string()
            } else {
                format!(" (revision: {revision})")
            };
            let _guard = self.failed_test(&revision, path, cmd, stderr);
            for error in errors {
                gha_error(error, &path.display().to_string(), &revision);
            }
        }
        Ok(())
    }
}

impl DuringTestRun for Gha {
    fn test_result(&mut self, _path: &Path, _revision: &str, _result: &TestResult) {}
}

/// Prints a human readable message as well as a github action workflow command where applicable.
pub struct TextAndGha;
impl StatusEmitter for TextAndGha {
    fn failed_test<'a>(
        &'a self,
        revision: &'a str,
        path: &'a Path,
        cmd: &'a Command,
        stderr: &'a [u8],
    ) -> Box<dyn Debug + 'a> {
        Box::new((
            Gha.failed_test(revision, path, cmd, stderr),
            Text.failed_test(revision, path, cmd, stderr),
        ))
    }

    fn run_tests(&self, _config: &Config) -> Box<dyn DuringTestRun> {
        Box::new(TextAndGha)
    }
}

impl DuringTestRun for TextAndGha {
    fn test_result(&mut self, path: &Path, revision: &str, result: &TestResult) {
        Text.test_result(path, revision, result);
        Gha.test_result(path, revision, result);
    }
}
