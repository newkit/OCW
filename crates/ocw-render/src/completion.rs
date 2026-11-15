//! Tab completion of the in-game console (spec §2.9), pure and Bevy-free.
//!
//! [`complete`] finds what the word before the cursor can become: a
//! command name, a subcommand, a long flag, or a value. Subcommands, flags
//! and `PossibleValue`s come from each command's clap definition; values
//! whose set is only known at run time (setting keys, world ids, weather
//! presets, …) come from a per-command *value provider* (the `values`
//! callback, fed by `ConsoleAppExt::console_values`), so the console hard
//! codes none of them. The caller passes only the commands this session
//! may run (the `Permission` filter).
//!
//! [`TabCompleter`] is the key behaviour (one rule for Tab):
//!
//! - one candidate: it is inserted, with a space after it;
//! - several: their longest common prefix is inserted and the candidates
//!   are listed under the input line; every further Tab inserts the next
//!   candidate in turn (Shift+Tab the previous one), the list stays with
//!   the inserted one marked;
//! - typing (any other change of the line) ends the cycle.
//!
//! Words with spaces are inserted in double quotes (the console's
//! [`split_line`](crate::console::split_line) groups them); a word typed
//! with an opening quote is completed inside it.

use clap::{Arg, Command};

/// What a value provider is asked.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueQuery<'a> {
    /// The command and the subcommands typed ("world switch").
    pub path: &'a str,
    /// The clap id of the argument whose value is completed.
    pub arg: &'a str,
    /// The values this argument already has (a list argument such as the
    /// `weather` words: the earlier ones).
    pub before: &'a [String],
    /// All complete words before the one completed (the command first).
    pub words: &'a [String],
}

/// The word under completion and what it can become.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Completion {
    /// Byte offset in the line where the word starts (its opening quote
    /// included).
    pub start: usize,
    /// The word as typed, without quotes.
    pub prefix: String,
    /// The word was started with a double quote.
    pub quoted: bool,
    /// The candidates that start with `prefix`, sorted, without
    /// duplicates.
    pub candidates: Vec<String>,
}

/// One word of a line, with where it starts.
#[derive(Debug, Clone, PartialEq)]
struct Word {
    text: String,
    start: usize,
    quoted: bool,
}

/// Splits `line` like [`crate::console::split_line`]: the complete words,
/// and the word the line ends in (empty after a space).
fn words(line: &str) -> (Vec<Word>, Word) {
    let mut done = Vec::new();
    let mut current: Option<Word> = None;
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.get_or_insert(Word {
                    text: String::new(),
                    start: i,
                    quoted: true,
                });
            }
            c if c.is_whitespace() && !in_quotes => {
                if let Some(w) = current.take() {
                    done.push(w);
                }
            }
            c => current
                .get_or_insert(Word {
                    text: String::new(),
                    start: i,
                    quoted: false,
                })
                .text
                .push(c),
        }
    }
    let last = current.unwrap_or(Word {
        text: String::new(),
        start: line.len(),
        quoted: false,
    });
    // A closed quote ("abc" ) is a complete word only after a space.
    (done, last)
}

/// Whether `w` is a number (a negative one is a value, not a flag).
fn is_number(w: &str) -> bool {
    w.parse::<f64>().is_ok()
}

fn takes_values(a: &Arg) -> bool {
    a.get_num_args().is_some_and(|r| r.takes_values())
}

fn max_values(a: &Arg) -> usize {
    a.get_num_args().map_or(1, |r| r.max_values())
}

/// The visible subcommands of `cmd` (clap's generated `help` left out).
fn subcommands(cmd: &Command) -> impl Iterator<Item = &Command> {
    cmd.get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
}

fn find_subcommand<'a>(cmd: &'a Command, word: &str) -> Option<&'a Command> {
    subcommands(cmd).find(|s| s.get_name() == word || s.get_all_aliases().any(|a| a == word))
}

/// The long flags of `cmd` (`--help` and `--version` left out).
fn long_flags(cmd: &Command) -> Vec<String> {
    cmd.get_arguments()
        .filter(|a| !a.is_positional() && !a.is_hide_set())
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        .filter_map(|a| a.get_long().map(|l| format!("--{l}")))
        .collect()
}

fn find_flag<'a>(cmd: &'a Command, word: &str) -> Option<&'a Arg> {
    if let Some(long) = word.strip_prefix("--") {
        let long = long.split('=').next().unwrap_or(long);
        cmd.get_arguments().find(|a| {
            a.get_long() == Some(long) || a.get_all_aliases().is_some_and(|v| v.contains(&long))
        })
    } else {
        let c = word.strip_prefix('-')?.chars().last()?;
        cmd.get_arguments().find(|a| a.get_short() == Some(c))
    }
}

/// An argument's clap `PossibleValue`s (the visible ones).
fn possible_values(a: &Arg) -> Vec<String> {
    a.get_possible_values()
        .iter()
        .filter(|p| !p.is_hide_set())
        .map(|p| p.get_name().to_owned())
        .collect()
}

/// The candidates for the word the `line` ends in. `commands`: (name,
/// definition) of the commands this session may run; `values`: the value
/// providers – `Some(list)` overrides the argument's `PossibleValue`s,
/// `None` keeps them.
pub fn complete(
    line: &str,
    commands: &[(&str, &Command)],
    values: &mut dyn FnMut(&ValueQuery) -> Option<Vec<String>>,
) -> Completion {
    let (done, current) = words(line);
    let mut out = Completion {
        start: current.start,
        prefix: current.text.clone(),
        quoted: current.quoted,
        candidates: Vec::new(),
    };
    let found = if done.is_empty() {
        commands.iter().map(|(n, _)| (*n).to_owned()).collect()
    } else {
        let texts: Vec<String> = done.iter().map(|w| w.text.clone()).collect();
        match commands.iter().find(|(n, _)| *n == texts[0]) {
            Some((_, cmd)) => {
                let mut cmd = (*cmd).clone();
                cmd.build();
                arguments(&cmd, &texts, &current.text, values)
            }
            None => Vec::new(),
        }
    };
    out.candidates = matching(found, &out.prefix);
    out
}

/// The candidates of `found` that start with `prefix`: case-sensitive
/// matches if there are any, else case-insensitive ones; sorted.
fn matching(found: Vec<String>, prefix: &str) -> Vec<String> {
    let mut exact: Vec<String> = found
        .iter()
        .filter(|c| c.starts_with(prefix))
        .cloned()
        .collect();
    if exact.is_empty() {
        let lower = prefix.to_lowercase();
        exact = found
            .into_iter()
            .filter(|c| c.to_lowercase().starts_with(&lower))
            .collect();
    }
    exact.sort();
    exact.dedup();
    exact
}

/// Walks the complete words after the command name through `cmd`'s
/// definition and returns what the next word can be.
fn arguments(
    root: &Command,
    texts: &[String],
    current: &str,
    values: &mut dyn FnMut(&ValueQuery) -> Option<Vec<String>>,
) -> Vec<String> {
    let mut cmd = root;
    let mut path = texts[0].clone();
    // The option waiting for its value, with the values it has.
    let mut option: Option<(&Arg, Vec<String>)> = None;
    // The positional being filled (by index) and its values.
    let mut pos = 0usize;
    let mut given: Vec<String> = Vec::new();
    let mut only_positionals = false;
    let positionals = |c: &Command| -> Vec<Arg> {
        let mut p: Vec<Arg> = c.get_positionals().cloned().collect();
        p.sort_by_key(|a| a.get_index().unwrap_or(usize::MAX));
        p
    };
    let mut pos_args = positionals(cmd);
    for w in &texts[1..] {
        if let Some((a, vals)) = &mut option {
            vals.push(w.clone());
            if vals.len() >= max_values(a) {
                option = None;
            }
            continue;
        }
        if w == "--" && !only_positionals {
            only_positionals = true;
            continue;
        }
        let flag_like = w.starts_with('-') && w.len() > 1 && !is_number(w);
        if flag_like
            && !only_positionals
            && let Some(a) = find_flag(cmd, w)
        {
            // `--name=value` and `-xVALUE` carry their value.
            if takes_values(a) && !w.contains('=') && (w.starts_with("--") || w.len() == 2) {
                option = Some((a, Vec::new()));
            }
            continue;
        }
        if pos == 0
            && given.is_empty()
            && let Some(sub) = find_subcommand(cmd, w)
        {
            cmd = sub;
            path.push(' ');
            path.push_str(sub.get_name());
            pos_args = positionals(cmd);
            continue;
        }
        given.push(w.clone());
        if let Some(a) = pos_args.get(pos)
            && given.len() >= max_values(a)
        {
            pos += 1;
            given.clear();
        }
    }
    let mut ask = |a: &Arg, before: &[String]| -> Vec<String> {
        let q = ValueQuery {
            path: &path,
            arg: a.get_id().as_str(),
            before,
            words: texts,
        };
        values(&q).unwrap_or_else(|| possible_values(a))
    };
    if let Some((a, vals)) = &option {
        return ask(a, vals);
    }
    let mut out = Vec::new();
    let value_arg = pos_args.get(pos);
    let hyphen_value = value_arg.is_some_and(|a| a.is_allow_hyphen_values_set());
    if current.starts_with('-') && !only_positionals && !hyphen_value {
        return long_flags(cmd);
    }
    if pos == 0 && given.is_empty() {
        out.extend(subcommands(cmd).map(|s| s.get_name().to_owned()));
    }
    if let Some(a) = value_arg {
        out.extend(ask(a, &given));
    }
    if current.starts_with('-') && !only_positionals {
        out.extend(long_flags(cmd));
    }
    out
}

/// Longest common prefix of `items` (by characters; case-insensitive when
/// `fold`, taking the first item's spelling).
fn common_prefix(items: &[String], fold: bool) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut n = first.chars().count();
    for s in &items[1..] {
        n = n.min(
            first
                .chars()
                .zip(s.chars())
                .take_while(|(a, b)| {
                    if fold {
                        a.to_lowercase().eq(b.to_lowercase())
                    } else {
                        a == b
                    }
                })
                .count(),
        );
    }
    first.chars().take(n).collect()
}

/// A candidate as it goes into the line: in quotes if it has spaces (or the
/// word was started with a quote); `open`: without the closing quote (a
/// common prefix, more to come).
fn quote(word: &str, quoted: bool, open: bool) -> String {
    if quoted || word.contains(char::is_whitespace) || (word.is_empty() && !open) {
        if open {
            format!("\"{word}")
        } else {
            format!("\"{word}\"")
        }
    } else {
        word.to_owned()
    }
}

/// Whether a completed word is followed by a space: not after a `field=`
/// or a directory (more of the same word follows).
fn ends_word(candidate: &str) -> bool {
    !(candidate.ends_with('=') || candidate.ends_with('/'))
}

/// The line after a Tab, and the list under it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabResult {
    /// The new line.
    pub line: String,
    /// The cursor in it (byte offset).
    pub cursor: usize,
    /// The candidates listed under the input line (empty: none listed).
    pub candidates: Vec<String>,
    /// The one inserted while cycling.
    pub selected: Option<usize>,
}

/// A running cycle: the line around the word and the candidates.
#[derive(Debug, Clone, PartialEq)]
struct Cycle {
    head: String,
    tail: String,
    quoted: bool,
    candidates: Vec<String>,
    selected: Option<usize>,
    /// The line and cursor this cycle last produced: another line means
    /// the user typed, and the cycle is over.
    line: String,
    cursor: usize,
}

/// The console's Tab key (see the module documentation for the rule).
#[derive(Debug, Clone, Default)]
pub struct TabCompleter {
    cycle: Option<Cycle>,
}

impl TabCompleter {
    /// Forgets the cycle (the line was changed, run or cleared).
    pub fn reset(&mut self) {
        self.cycle = None;
    }

    /// Whether a cycle is running for this `line` and `cursor`.
    pub fn cycling(&self, line: &str, cursor: usize) -> bool {
        self.cycle
            .as_ref()
            .is_some_and(|c| c.line == line && c.cursor == cursor)
    }

    /// Tab (`backwards`: Shift+Tab) on `line` with the cursor at byte
    /// `cursor`. `complete` is asked for the candidates of the text before
    /// the cursor when no cycle runs.
    pub fn tab(
        &mut self,
        line: &str,
        cursor: usize,
        backwards: bool,
        complete: impl FnOnce(&str) -> Completion,
    ) -> TabResult {
        let cursor = cursor.min(line.len());
        if self.cycling(line, cursor) {
            let c = self.cycle.as_mut().expect("cycling");
            let n = c.candidates.len();
            let next = match (c.selected, backwards) {
                (None, false) => 0,
                (None, true) => n - 1,
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
            };
            c.selected = Some(next);
            let word = &c.candidates[next];
            let mut text = quote(word, c.quoted, false);
            if ends_word(word) && !c.tail.starts_with(char::is_whitespace) {
                text.push(' ');
            }
            c.line = format!("{}{text}{}", c.head, c.tail);
            c.cursor = c.head.len() + text.len();
            return TabResult {
                line: c.line.clone(),
                cursor: c.cursor,
                candidates: c.candidates.clone(),
                selected: c.selected,
            };
        }
        self.cycle = None;
        let (before, tail) = line.split_at(cursor);
        let comp = complete(before);
        let head = &before[..comp.start.min(before.len())];
        let unchanged = TabResult {
            line: line.to_owned(),
            cursor,
            candidates: Vec::new(),
            selected: None,
        };
        match comp.candidates.len() {
            0 => unchanged,
            1 => {
                let word = &comp.candidates[0];
                let mut text = quote(word, comp.quoted, false);
                if ends_word(word) && !tail.starts_with(char::is_whitespace) {
                    text.push(' ');
                }
                TabResult {
                    line: format!("{head}{text}{tail}"),
                    cursor: head.len() + text.len(),
                    candidates: Vec::new(),
                    selected: None,
                }
            }
            _ => {
                let fold = !comp.candidates.iter().all(|c| c.starts_with(&comp.prefix));
                let common = common_prefix(&comp.candidates, fold);
                let grows = common.chars().count() > comp.prefix.chars().count();
                let quoted = comp.quoted;
                let text = if grows {
                    quote(&common, quoted, true)
                } else {
                    line[head.len()..cursor].to_owned()
                };
                let new_line = format!("{head}{text}{tail}");
                let new_cursor = head.len() + text.len();
                self.cycle = Some(Cycle {
                    head: head.to_owned(),
                    tail: tail.to_owned(),
                    quoted,
                    candidates: comp.candidates.clone(),
                    selected: None,
                    line: new_line.clone(),
                    cursor: new_cursor,
                });
                TabResult {
                    line: new_line,
                    cursor: new_cursor,
                    candidates: comp.candidates,
                    selected: None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{Arg, ArgAction, Command};

    use super::*;

    fn commands() -> Vec<(&'static str, Command)> {
        vec![
            ("help", Command::new("help").arg(Arg::new("command"))),
            (
                "hud",
                Command::new("hud").arg(Arg::new("on").value_parser(["on", "off"])),
            ),
            (
                "hud.profile",
                Command::new("hud.profile").arg(Arg::new("on").value_parser(["on", "off"])),
            ),
            (
                "hud.units",
                Command::new("hud.units").arg(
                    Arg::new("units")
                        .required(true)
                        .value_parser(["metric", "imperial"]),
                ),
            ),
            (
                "world",
                Command::new("world")
                    .subcommand_required(true)
                    .subcommand(Command::new("list"))
                    .subcommand(Command::new("info"))
                    .subcommand(Command::new("switch").arg(Arg::new("world").required(true))),
            ),
            (
                "set",
                Command::new("set")
                    .arg(Arg::new("temp").long("temp").action(ArgAction::SetTrue))
                    .arg(Arg::new("key").required(true))
                    .arg(Arg::new("value").required(true).num_args(1..)),
            ),
            (
                "weather",
                Command::new("weather").arg(
                    Arg::new("args")
                        .num_args(0..)
                        .allow_hyphen_values(true)
                        .action(ArgAction::Append),
                ),
            ),
            (
                "camera.path",
                Command::new("camera.path")
                    .arg(
                        Arg::new("action")
                            .required(true)
                            .value_parser(["add", "list", "load", "play", "save"]),
                    )
                    .arg(Arg::new("args").num_args(0..).allow_hyphen_values(true))
                    .arg(Arg::new("seconds").long("seconds"))
                    .arg(
                        Arg::new("ease")
                            .long("ease")
                            .value_parser(["linear", "smooth", "in-out"]),
                    ),
            ),
        ]
    }

    /// The value providers of the tests: setting keys and values, world
    /// ids (one with a space), weather words that depend on the earlier
    /// ones.
    fn provider(q: &ValueQuery) -> Option<Vec<String>> {
        let v = |s: &[&str]| Some(s.iter().map(|x| (*x).to_owned()).collect());
        match (q.path, q.arg) {
            ("set", "key") => v(&["display.hud", "display.units", "ride.bots"]),
            ("set", "value") => {
                match q.words.iter().find(|w| !w.starts_with("--") && *w != "set") {
                    Some(k) if k == "display.units" => v(&["metric", "imperial"]),
                    Some(k) if k == "display.hud" => v(&["on", "off"]),
                    _ => None,
                }
            }
            ("help", "command") => v(&["help", "hud", "hud.profile", "hud.units", "world"]),
            ("world switch", "world") => v(&["liechtenstein", "loop", "hills", "Black Forest"]),
            ("weather", "args") => match q.before {
                [] => v(&["clear", "cloudy", "rain", "real", "clouds=", "list"]),
                [w] if w == "real" => v(&["automatic", "dwd", "metno"]),
                _ => v(&[]),
            },
            _ => None,
        }
    }

    fn comp(line: &str) -> Completion {
        let cmds = commands();
        let refs: Vec<(&str, &Command)> = cmds.iter().map(|(n, c)| (*n, c)).collect();
        complete(line, &refs, &mut provider)
    }

    fn cands(line: &str) -> Vec<String> {
        comp(line).candidates
    }

    fn tab(t: &mut TabCompleter, line: &str) -> TabResult {
        t.tab(line, line.len(), false, comp)
    }

    #[test]
    fn command_names_by_prefix() {
        assert_eq!(cands("hud.u"), ["hud.units"]);
        assert_eq!(cands("hu"), ["hud", "hud.profile", "hud.units"]);
        assert_eq!(cands("").len(), commands().len());
        assert!(cands("nope").is_empty());
        assert!(
            cands("nope ").is_empty(),
            "no arguments of unknown commands"
        );
        // Case-insensitive when nothing matches exactly.
        assert_eq!(cands("HUD.U"), ["hud.units"]);
    }

    #[test]
    fn the_prefix_and_a_single_candidate() {
        let mut t = TabCompleter::default();
        let r = tab(&mut t, "hud.u");
        assert_eq!((r.line.as_str(), r.cursor), ("hud.units ", 10));
        assert!(r.candidates.is_empty());
        // The longest common prefix first.
        let r = tab(&mut t, "wo");
        assert_eq!(r.line, "world ");
        let r = tab(&mut t, "hud.");
        assert_eq!(r.line, "hud.");
        assert_eq!(r.candidates, ["hud.profile", "hud.units"]);
        // Nothing to complete: the line stays.
        let r = tab(&mut t, "xyz");
        assert_eq!((r.line.as_str(), r.cursor), ("xyz", 3));
        assert!(r.candidates.is_empty());
    }

    /// Several candidates: the common prefix and the list, then Tab cycles
    /// through them, Shift+Tab back; typing ends the cycle.
    #[test]
    fn ambiguity_lists_then_cycles() {
        let mut t = TabCompleter::default();
        let r = tab(&mut t, "h");
        assert_eq!(r.line, "h");
        assert_eq!(r.candidates, ["help", "hud", "hud.profile", "hud.units"]);
        assert_eq!(r.selected, None);
        let r = tab(&mut t, &r.line);
        assert_eq!((r.line.as_str(), r.selected), ("help ", Some(0)));
        let r = tab(&mut t, &r.line);
        assert_eq!((r.line.as_str(), r.selected), ("hud ", Some(1)));
        let r = t.tab(&r.line, r.cursor, true, comp);
        assert_eq!((r.line.as_str(), r.selected), ("help ", Some(0)));
        let r = t.tab(&r.line, r.cursor, true, comp);
        assert_eq!(r.line, "hud.units ", "Shift+Tab wraps around");
        assert_eq!(r.candidates.len(), 4, "the list stays while cycling");
        let r = tab(&mut t, &r.line);
        assert_eq!(r.line, "help ", "Tab wraps around");
        // Typing ends the cycle: a new completion of the new line.
        let r = tab(&mut t, "help hud.");
        assert_eq!(r.line, "help hud.");
        assert_eq!(r.candidates, ["hud.profile", "hud.units"]);
        // The common prefix grows first, then the list cycles.
        let mut t = TabCompleter::default();
        let r = tab(&mut t, "hud.p");
        assert_eq!(r.line, "hud.profile ");
        let r = tab(&mut t, "hu");
        assert_eq!(r.line, "hud");
        assert_eq!(r.candidates.len(), 3);
        let r = tab(&mut t, &r.line);
        assert_eq!(r.line, "hud ");
    }

    #[test]
    fn subcommands_flags_and_values() {
        assert_eq!(cands("world "), ["info", "list", "switch"]);
        assert_eq!(cands("world s"), ["switch"]);
        assert_eq!(cands("world switch l"), ["liechtenstein", "loop"]);
        assert!(cands("world list ").is_empty());
        // PossibleValues.
        assert_eq!(cands("hud.units "), ["imperial", "metric"]);
        assert_eq!(cands("hud o"), ["off", "on"]);
        assert!(cands("hud on ").is_empty(), "one value only");
        // Long flags (clap's --help left out), and an option's values.
        assert_eq!(cands("set --"), ["--temp"]);
        assert_eq!(cands("camera.path play --"), ["--ease", "--seconds"]);
        assert_eq!(
            cands("camera.path play --ease "),
            ["in-out", "linear", "smooth"]
        );
        assert_eq!(
            cands("camera.path "),
            ["add", "list", "load", "play", "save"]
        );
        // Values from a provider, which may look at the earlier words.
        assert_eq!(cands("set display.u"), ["display.units"]);
        assert_eq!(cands("set --temp display.h"), ["display.hud"]);
        assert_eq!(cands("set display.units "), ["imperial", "metric"]);
        assert_eq!(cands("set --temp display.hud o"), ["off", "on"]);
        assert_eq!(cands("weather r"), ["rain", "real"]);
        assert_eq!(cands("weather real "), ["automatic", "dwd", "metno"]);
        // A field= is completed without a space after it.
        let mut t = TabCompleter::default();
        assert_eq!(tab(&mut t, "weather clo").line, "weather cloud");
        assert_eq!(tab(&mut t, "weather clouds").line, "weather clouds=");
        // The words after the cursor stay.
        let mut t = TabCompleter::default();
        let r = t.tab("hud.u metric", 5, false, comp);
        assert_eq!((r.line.as_str(), r.cursor), ("hud.units metric", 9));
    }

    #[test]
    fn quoted_arguments() {
        let (done, last) = words("echo \"a b\" \"c d");
        assert_eq!(
            done.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            ["echo", "a b"]
        );
        assert_eq!(
            (last.text.as_str(), last.start, last.quoted),
            ("c d", 11, true)
        );
        // A candidate with a space goes in quotes.
        let mut t = TabCompleter::default();
        assert_eq!(
            tab(&mut t, "world switch B").line,
            "world switch \"Black Forest\" "
        );
        // Completed inside an opened quote.
        let mut t = TabCompleter::default();
        assert_eq!(cands("world switch \"Bla"), ["Black Forest"]);
        assert_eq!(
            tab(&mut t, "world switch \"Black F").line,
            "world switch \"Black Forest\" "
        );
        // A quoted word counts as one word for what follows.
        assert_eq!(cands("set \"display.units\" "), ["imperial", "metric"]);
    }

    /// The caller passes only the commands the session may run: the
    /// others are never offered (a remote server: no dev commands).
    #[test]
    fn the_permission_filter() {
        let all = commands();
        let dev = Command::new("dev.thing");
        let mut refs: Vec<(&str, &Command)> = all.iter().map(|(n, c)| (*n, c)).collect();
        assert!(complete("dev", &refs, &mut provider).candidates.is_empty());
        refs.push(("dev.thing", &dev));
        assert_eq!(
            complete("dev", &refs, &mut provider).candidates,
            ["dev.thing"]
        );
    }

    #[test]
    fn negative_numbers_are_values() {
        let c = Command::new("camera.orbit")
            .arg(Arg::new("yaw").allow_negative_numbers(true))
            .arg(Arg::new("pitch").value_parser(["10", "20"]));
        let refs = [("camera.orbit", &c)];
        assert_eq!(
            complete("camera.orbit -45 ", &refs, &mut provider).candidates,
            ["10", "20"]
        );
    }
}
