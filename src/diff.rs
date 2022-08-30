use colored::*;
use diff::{chars, lines, Result, Result::*};

#[derive(Default)]
struct DiffState<'a> {
    /// Whether we've already printed something, so we should print starting context, too.
    print_start_context: bool,
    /// When we skip lines, remember the last `CONTEXT` ones to
    /// display after the "skipped N lines" message
    skipped_lines: Vec<&'a str>,
    /// When we see a removed line, we don't print it, we
    /// keep it around to compare it with the next added line.
    prev_left: Option<&'a str>,
}

/// How many lines of context are displayed around the actual diffs
const CONTEXT: usize = 2;

impl<'a> DiffState<'a> {
    /// Print `... n lines skipped ...` followed by the last `CONTEXT` lines.
    fn print_end_skip(&self, skipped: usize) {
        self.print_skipped_msg(skipped);
        for line in self.skipped_lines.iter().rev().take(CONTEXT).rev() {
            eprintln!(" {line}");
        }
    }
    fn print_skipped_msg(&self, skipped: usize) {
        match skipped {
            0 => {}
            1 => eprintln!(" {}", self.skipped_lines[CONTEXT]),
            _ => eprintln!("... {skipped} lines skipped ..."),
        }
    }
    fn print_start_skip(&self) {
        for line in self.skipped_lines.iter().take(CONTEXT) {
            eprintln!(" {line}");
        }
    }
    fn print_skip(&mut self) {
        let half = self.skipped_lines.len() / 2;
        if !self.print_start_context {
            self.print_start_context = true;
            self.print_end_skip(self.skipped_lines.len().saturating_sub(CONTEXT));
        } else if half < CONTEXT {
            for line in self.skipped_lines.drain(..) {
                eprintln!(" {line}");
            }
        } else {
            self.print_start_skip();
            let skipped = self.skipped_lines.len() - CONTEXT * 2;
            self.print_end_skip(skipped);
        }
        self.skipped_lines.clear();
    }

    fn skip(&mut self, line: &'a str) {
        self.skipped_lines.push(line);
    }

    fn print_prev(&mut self) {
        if let Some(l) = self.prev_left.take() {
            self.print_left(l);
        }
    }

    fn print_left(&self, l: &str) {
        eprintln!("{}{}", "-".red(), l.red());
    }

    fn print_right(&self, r: &str) {
        eprintln!("{}{}", "+".green(), r.green());
    }

    fn row(&mut self, row: Result<&'a str>) {
        match row {
            Left(l) => {
                self.print_skip();
                self.print_prev();
                self.prev_left = Some(l);
            }
            Both(l, _) => {
                self.print_prev();
                self.skip(l);
            }
            Right(r) => {
                if let Some(l) = self.prev_left.take() {
                    // If the lines only add chars or only remove chars, display an inline diff
                    let diff = chars(l, r);
                    let mut seen_l = false;
                    let mut seen_r = false;
                    for char in &diff {
                        match char {
                            Left(l) if !l.is_whitespace() => seen_l = true,
                            Right(r) if !r.is_whitespace() => seen_r = true,
                            _ => {}
                        }
                    }
                    if seen_l && seen_r {
                        // the line both adds and removes chars, print both lines, but highlight their differences
                        eprint!("{}", "-".red());
                        for char in &diff {
                            match char {
                                Left(l) => eprint!("{}", l.to_string().red()),
                                Right(_) => {}
                                Both(l, _) => eprint!("{}", l),
                            }
                        }
                        eprintln!();
                        eprint!("{}", "+".green());
                        for char in &diff {
                            match char {
                                Left(_) => {}
                                Right(r) => eprint!("{}", r.to_string().green()),
                                Both(l, _) => eprint!("{}", l),
                            }
                        }
                        eprintln!();
                    } else {
                        eprint!("{}", "~".yellow());
                        for char in diff {
                            match char {
                                Left(l) => eprint!("{}", l.to_string().red()),
                                Both(l, _) => eprint!("{}", l),
                                Right(r) => eprint!("{}", r.to_string().green()),
                            }
                        }
                        eprintln!();
                    }
                } else {
                    self.print_skip();
                    self.print_right(r);
                }
            }
        }
    }

    fn finish(self) {
        self.print_start_skip();
        self.print_skipped_msg(self.skipped_lines.len().saturating_sub(CONTEXT));
        eprintln!()
    }
}

pub fn print_diff(expected: &str, actual: &str) {
    let mut state = DiffState::default();
    for row in lines(expected, actual) {
        state.row(row);
    }
    state.finish();
}
