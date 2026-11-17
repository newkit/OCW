//! In-game console (spec §2.9).
//!
//! Plugins register commands with [`ConsoleAppExt::console_command`]. Each
//! command has a clap argument definition, help text, a permission level and a
//! handler that gets exclusive access to the [`World`]. Commands can be typed in
//! the overlay (toggle with `` ` `` or F12), queued programmatically with
//! [`ConsoleState::queue`], or run from a file with `exec <file>`.
//!
//! Tab completes the word before the cursor (command names, subcommands,
//! long flags, values; [`crate::completion`] has the rule): from the clap
//! definitions, and from the value providers commands register with
//! [`ConsoleAppExt::console_values`] for values only known at run time.
//! Only commands the session may run are offered.

use std::collections::{BTreeMap, VecDeque};

use bevy::app::AppExit;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::view::screenshot::Screenshot;
use bevy::window::{PrimaryWindow, WindowRef};
use bevy_egui::{
    EguiContext, EguiContexts, EguiPlugin, EguiPrimaryContextPass, EguiSchedule,
    PrimaryEguiContext, egui,
};
use clap::{Arg, ArgMatches, Command, value_parser};

use crate::camera::{
    CameraMode, CameraOrbit, CameraTilt, MAX_DISTANCE, MAX_PITCH, MIN_DISTANCE, MIN_PITCH,
};
use crate::completion::{TabCompleter, ValueQuery};
use crate::hud::{ShowFps, ShowGps, ShowHud, ShowProfile, Units};

/// Console with its egui overlay (toggle with `` ` `` or F12).
pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin::default());
        }
        app.add_plugins(ConsoleCorePlugin)
            .add_systems(Update, toggle_console.before(run_pending_commands))
            .add_systems(PostUpdate, console_on_top)
            .add_systems(EguiPrimaryContextPass, console_ui);
    }
}

/// The camera the console (the primary egui context) draws on: the active
/// camera with the highest order that renders into the primary window.
/// While the workshop is open the world camera renders into the small ride
/// window (an image) and the workshop camera is the one on screen, so the
/// console is drawn over the workshop, not into the ride window (spec §14
/// Phase 4, 4M). `cams`: (entity, order, active, renders to the primary
/// window).
pub fn console_host(cams: impl IntoIterator<Item = (Entity, isize, bool, bool)>) -> Option<Entity> {
    cams.into_iter()
        .filter(|(_, _, active, window)| *active && *window)
        .max_by_key(|(e, order, _, _)| (*order, std::cmp::Reverse(*e)))
        .map(|(e, ..)| e)
}

/// Keeps the primary egui context on [`console_host`]: a fresh one on the
/// camera that becomes the one on screen (bevy_egui keeps a context's
/// textures – the font atlas – per camera entity: a moved context only sent
/// partial updates of an atlas the new camera never had, and drew nothing),
/// also when its camera was despawned (the workshop's, on closing). The old
/// camera keeps bevy_egui's (now empty) output components: its render
/// entity links an egui view that is only refreshed while they are there
/// (removing them left a stale link, and bevy_egui's transforms panicked).
#[allow(clippy::type_complexity)] // a Bevy system query
fn console_on_top(
    mut commands: Commands,
    cams: Query<(Entity, &Camera, Option<&RenderTarget>)>,
    hosts: Query<Entity, With<PrimaryEguiContext>>,
    primary: Query<Entity, With<PrimaryWindow>>,
) {
    let primary = primary.single().ok();
    let to_window = |t: Option<&RenderTarget>| match t {
        None | Some(RenderTarget::Window(WindowRef::Primary)) => true,
        Some(RenderTarget::Window(WindowRef::Entity(e))) => Some(*e) == primary,
        _ => false,
    };
    let Some(host) = console_host(
        cams.iter()
            .map(|(e, c, t)| (e, c.order, c.is_active, to_window(t))),
    ) else {
        return;
    };
    if hosts.iter().any(|e| e == host) && hosts.iter().count() == 1 {
        return;
    }
    for e in &hosts {
        if e != host {
            commands
                .entity(e)
                .remove::<(PrimaryEguiContext, EguiContext, EguiSchedule)>();
        }
    }
    if !hosts.iter().any(|e| e == host) {
        commands
            .entity(host)
            .insert((EguiContext::default(), PrimaryEguiContext));
    }
}

/// Command registry, state and execution without any UI or input handling.
/// Usable headless (tests, `--cmd` automation).
pub struct ConsoleCorePlugin;

impl Plugin for ConsoleCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleCommands>()
            .init_resource::<ConsoleValues>()
            .init_resource::<ConsoleState>()
            .init_resource::<ShowFps>()
            .init_resource::<ShowProfile>()
            .init_resource::<ShowGps>()
            .init_resource::<ShowHud>()
            .init_resource::<Units>()
            .init_resource::<CameraMode>()
            .init_resource::<CameraOrbit>()
            .init_resource::<crate::camera::SkyLook>()
            .init_resource::<CameraTilt>()
            .init_resource::<crate::drone_camera::DroneCamera>()
            .add_systems(Update, (run_tab_request, run_pending_commands).chain());
        register_builtin_commands(app);
    }
}

/// Who may run a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    /// Always available.
    User,
    /// Only with a local server or in debug builds.
    Dev,
    /// Organizers / staff on a remote server; executed on the server.
    Admin,
}

/// A command handler. Returns the text to print, or an error message.
pub type Handler = fn(&mut World, &ArgMatches) -> Result<String, String>;

pub struct ConsoleCommand {
    pub level: Permission,
    pub command: Command,
    pub handler: Handler,
}

impl ConsoleCommand {
    /// A command with the given clap definition (its name is the command name).
    pub fn new(level: Permission, command: Command, handler: Handler) -> Self {
        Self {
            level,
            command: command.no_binary_name(false),
            handler,
        }
    }
}

#[derive(Resource, Default)]
pub struct ConsoleCommands(BTreeMap<String, ConsoleCommand>);

/// A value provider for Tab completion: the values an argument can take
/// now (setting keys, world ids, presets, …), unfiltered – the console
/// keeps the ones that match the typed word. It may look at the words
/// before ([`ValueQuery`]).
pub type ValueProvider = fn(&mut World, &ValueQuery) -> Vec<String>;

/// The value providers by (command path, argument id).
#[derive(Resource, Default)]
pub struct ConsoleValues(BTreeMap<(String, String), ValueProvider>);

pub trait ConsoleAppExt {
    fn console_command(&mut self, command: ConsoleCommand) -> &mut Self;

    /// Completes the values of argument `arg` (its clap id) of `command`
    /// (the command with its subcommands: `"world switch"`) with
    /// `provider` instead of the argument's `PossibleValue`s.
    fn console_values(&mut self, command: &str, arg: &str, provider: ValueProvider) -> &mut Self;
}

impl ConsoleAppExt for App {
    fn console_command(&mut self, command: ConsoleCommand) -> &mut Self {
        let name = command.command.get_name().to_owned();
        self.world_mut()
            .get_resource_or_insert_with(ConsoleCommands::default)
            .0
            .insert(name, command);
        self
    }

    fn console_values(&mut self, command: &str, arg: &str, provider: ValueProvider) -> &mut Self {
        self.world_mut()
            .get_resource_or_insert_with(ConsoleValues::default)
            .0
            .insert((command.to_owned(), arg.to_owned()), provider);
        self
    }
}

#[derive(Resource)]
pub struct ConsoleState {
    open: bool,
    input: String,
    log: Vec<String>,
    history: Vec<String>,
    history_pos: Option<usize>,
    pending: VecDeque<String>,
    /// Highest permission level the current session allows.
    pub permission: Permission,
    /// Tab completion: the cycle, the candidates listed under the input
    /// line and the one inserted.
    completer: TabCompleter,
    candidates: Vec<String>,
    selected: Option<usize>,
    /// A Tab to complete next frame: (cursor in bytes, Shift held).
    tab_request: Option<(usize, bool)>,
    /// Where the input line's cursor goes (bytes) after a completion.
    cursor_to: Option<usize>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            open: false,
            input: String::new(),
            log: vec!["Open Cycling World console – type 'help'".into()],
            history: Vec::new(),
            history_pos: None,
            pending: VecDeque::new(),
            permission: Permission::User,
            completer: TabCompleter::default(),
            candidates: Vec::new(),
            selected: None,
            tab_request: None,
            cursor_to: None,
        }
    }
}

impl ConsoleState {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Queues a command line to run in the next frame.
    pub fn queue(&mut self, line: impl Into<String>) {
        self.pending.push_back(line.into());
    }

    /// Appends a line to the console output (also logged).
    pub fn print(&mut self, line: impl Into<String>) {
        let line = line.into();
        info!(target: "console", "{line}");
        self.log.push(line);
        if self.log.len() > 500 {
            self.log.drain(..100);
        }
    }

    pub fn log(&self) -> &[String] {
        &self.log
    }

    /// The command line being typed.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Replaces the command line (the cursor goes to its end); ends a
    /// completion cycle.
    pub fn set_input(&mut self, line: impl Into<String>) {
        self.input = line.into();
        self.cursor_to = Some(self.input.len());
        self.end_completion();
    }

    /// The completion candidates listed under the input line, and the one
    /// inserted while cycling through them.
    pub fn candidates(&self) -> (&[String], Option<usize>) {
        (&self.candidates, self.selected)
    }

    /// Tab (Shift+Tab: `backwards`) with the cursor at byte `cursor` of
    /// the input line (`None`: at its end), completed next frame.
    pub fn request_tab(&mut self, cursor: Option<usize>, backwards: bool) {
        self.tab_request = Some((cursor.unwrap_or(self.input.len()), backwards));
    }

    fn end_completion(&mut self) {
        self.completer.reset();
        self.candidates.clear();
        self.selected = None;
    }
}

/// The commands this session may run (name, clap definition).
fn allowed_commands(world: &World) -> Vec<(String, Command)> {
    let allowed = world.resource::<ConsoleState>().permission;
    world
        .resource::<ConsoleCommands>()
        .0
        .iter()
        .filter(|(_, c)| c.level <= allowed)
        .map(|(n, c)| (n.clone(), c.command.clone()))
        .collect()
}

/// Tab on the console's input line at byte `cursor` (see
/// [`crate::completion`]): the line, the cursor and the listed candidates
/// change in [`ConsoleState`].
pub fn tab_complete(world: &mut World, cursor: usize, backwards: bool) {
    let (line, mut completer) = {
        let mut s = world.resource_mut::<ConsoleState>();
        (s.input.clone(), std::mem::take(&mut s.completer))
    };
    let cursor = (0..=cursor.min(line.len()))
        .rev()
        .find(|&i| line.is_char_boundary(i))
        .unwrap_or(0);
    let r = completer.tab(&line, cursor, backwards, |before| {
        let commands = allowed_commands(world);
        let refs: Vec<(&str, &Command)> = commands.iter().map(|(n, c)| (n.as_str(), c)).collect();
        let providers = world.resource::<ConsoleValues>().0.clone();
        crate::completion::complete(before, &refs, &mut |q| {
            let p = providers.get(&(q.path.to_owned(), q.arg.to_owned()))?;
            Some(p(world, q))
        })
    });
    let mut s = world.resource_mut::<ConsoleState>();
    s.completer = completer;
    s.input = r.line;
    s.cursor_to = Some(r.cursor);
    s.candidates = r.candidates;
    s.selected = r.selected;
}

/// Runs the Tab the overlay saw last frame.
fn run_tab_request(world: &mut World) {
    if let Some((cursor, backwards)) = world.resource_mut::<ConsoleState>().tab_request.take() {
        tab_complete(world, cursor, backwards);
    }
}

/// Splits a command line into words; double quotes group words.
pub fn split_line(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut has_word = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has_word = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has_word {
                    words.push(std::mem::take(&mut current));
                    has_word = false;
                }
            }
            c => {
                current.push(c);
                has_word = true;
            }
        }
    }
    if has_word {
        words.push(current);
    }
    words
}

fn run_pending_commands(world: &mut World) {
    while let Some(line) = world.resource_mut::<ConsoleState>().pending.pop_front() {
        let words = split_line(&line);
        if words.is_empty() || words[0].starts_with('#') {
            continue;
        }
        world
            .resource_mut::<ConsoleState>()
            .print(format!("> {line}"));
        let result = execute(world, &words);
        let mut state = world.resource_mut::<ConsoleState>();
        match result {
            Ok(out) if out.is_empty() => {}
            Ok(out) => out.lines().for_each(|l| state.print(l)),
            Err(err) => err.lines().for_each(|l| state.print(format!("error: {l}"))),
        }
    }
}

fn execute(world: &mut World, words: &[String]) -> Result<String, String> {
    let name = &words[0];
    let found = world
        .resource::<ConsoleCommands>()
        .0
        .get(name)
        .map(|c| (c.level, c.command.clone(), c.handler));
    let Some((level, command, handler)) = found else {
        return Err(format!("unknown command '{name}' – type 'help'"));
    };
    let allowed = world.resource::<ConsoleState>().permission;
    if level > allowed {
        return Err(format!(
            "'{name}' needs {level:?} permission (this session allows {allowed:?})"
        ));
    }
    let matches = command
        .try_get_matches_from(words)
        .map_err(|e| e.render().to_string().trim_end().to_owned())?;
    handler(world, &matches)
}

fn toggle_console(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<ConsoleState>) {
    if keys.just_pressed(KeyCode::Backquote) || keys.just_pressed(KeyCode::F12) {
        state.open = !state.open;
        state.input.retain(|c| c != '`');
    }
}

/// The input line's egui id (its cursor is set after a completion).
const INPUT_ID: &str = "ocw-console-input";

/// How many completion candidates the list under the input line shows.
const MAX_LISTED: usize = 80;

fn console_ui(mut contexts: EguiContexts, mut state: ResMut<ConsoleState>) -> Result {
    if !state.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let width = ctx.content_rect().width() - 24.0;
    let state = &mut *state;
    let listing = !state.candidates.is_empty();
    let input_id = egui::Id::new(INPUT_ID);
    // After a completion (or `console input`): the cursor where it ends.
    if let Some(at) = state.cursor_to.take() {
        let at = if state.input.is_char_boundary(at) {
            at
        } else {
            state.input.len()
        };
        let chars = state.input[..at].chars().count();
        let mut edit = egui::text_edit::TextEditState::load(ctx, input_id).unwrap_or_default();
        edit.cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(chars),
            )));
        edit.store(ctx, input_id);
    }
    egui::Window::new("Console")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 8.0])
        .fixed_size([width, if listing { 380.0 } else { 300.0 }])
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(220))
                .inner_margin(8.0)
                .corner_radius(6.0),
        )
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &state.log {
                        ui.label(
                            egui::RichText::new(line)
                                .monospace()
                                .color(egui::Color32::LIGHT_GRAY),
                        );
                    }
                });
            // Tab and Shift+Tab stay in the line (no focus change, no tab
            // character): they complete.
            let output = egui::TextEdit::singleline(&mut state.input)
                .id(input_id)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .hint_text("command (Tab completes)")
                .event_filter(egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                })
                .show(ui);
            let response = &output.response.response;
            response.request_focus();
            let typed = response.changed();
            state.input.retain(|c| c != '`');
            let cursor_chars = output
                .cursor_range
                .map_or(usize::MAX, |r| r.primary.index.0);
            let (enter, up, down, tab, shift) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::Tab),
                    i.modifiers.shift,
                )
            });
            if typed {
                state.end_completion();
            }
            if enter && !state.input.trim().is_empty() {
                let line = std::mem::take(&mut state.input);
                state.history.push(line.clone());
                state.history_pos = None;
                state.pending.push_back(line);
                state.end_completion();
            } else if tab {
                let cursor = state
                    .input
                    .char_indices()
                    .nth(cursor_chars)
                    .map_or(state.input.len(), |(b, _)| b);
                state.request_tab(Some(cursor), shift);
            } else if up && !state.history.is_empty() {
                let pos = state
                    .history_pos
                    .map_or(state.history.len() - 1, |p| p.saturating_sub(1));
                state.history_pos = Some(pos);
                state.input = state.history[pos].clone();
                state.end_completion();
            } else if down && let Some(p) = state.history_pos {
                if p + 1 < state.history.len() {
                    state.history_pos = Some(p + 1);
                    state.input = state.history[p + 1].clone();
                } else {
                    state.history_pos = None;
                    state.input.clear();
                }
                state.end_completion();
            }
            if listing {
                candidate_list(ui, &state.candidates, state.selected);
            }
        });
    Ok(())
}

/// The completion candidates under the input line, the inserted one
/// highlighted.
fn candidate_list(ui: &mut egui::Ui, candidates: &[String], selected: Option<usize>) {
    egui::ScrollArea::vertical()
        .id_salt("ocw-console-candidates")
        .max_height(64.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (i, c) in candidates.iter().take(MAX_LISTED).enumerate() {
                    let text = egui::RichText::new(c).monospace();
                    let text = if Some(i) == selected {
                        text.color(egui::Color32::BLACK)
                            .background_color(egui::Color32::from_rgb(240, 200, 80))
                    } else {
                        text.color(egui::Color32::from_rgb(150, 200, 255))
                    };
                    ui.label(text);
                }
                if candidates.len() > MAX_LISTED {
                    ui.label(
                        egui::RichText::new(format!("… {} more", candidates.len() - MAX_LISTED))
                            .monospace()
                            .color(egui::Color32::GRAY),
                    );
                }
            });
        });
}

fn register_builtin_commands(app: &mut App) {
    use Permission::*;
    app.console_command(ConsoleCommand::new(
        User,
        Command::new("help")
            .about("List commands, or show help for one command")
            .arg(Arg::new("command")),
        |world, m| {
            let commands = world.resource::<ConsoleCommands>();
            if let Some(name) = m.get_one::<String>("command") {
                let c = commands
                    .0
                    .get(name)
                    .ok_or(format!("unknown command '{name}'"))?;
                return Ok(c.command.clone().render_long_help().to_string());
            }
            let allowed = world.resource::<ConsoleState>().permission;
            Ok(commands
                .0
                .iter()
                .filter(|(_, c)| c.level <= allowed)
                .map(|(name, c)| {
                    let about = c
                        .command
                        .get_about()
                        .map(|a| a.to_string())
                        .unwrap_or_default();
                    format!("{name:<20} {about}")
                })
                .collect::<Vec<_>>()
                .join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("console")
            .about("Open, close or toggle the console overlay; 'input TEXT' types TEXT into it, 'tab' presses Tab (scripts, tests)")
            .arg(
                Arg::new("action")
                    .value_parser(["open", "close", "toggle", "input", "tab"])
                    .default_value("toggle"),
            )
            .arg(Arg::new("text").help("input: the command line to type")),
        |world, m| {
            let action = m.get_one::<String>("action").map(String::as_str);
            if action == Some("tab") {
                // After this command's own line went into the log.
                let at = world.resource::<ConsoleState>().input.len();
                tab_complete(world, at, false);
                world.resource_mut::<ConsoleState>().open = true;
                return Ok(String::new());
            }
            let mut state = world.resource_mut::<ConsoleState>();
            state.open = match action {
                Some("open") => true,
                Some("close") => false,
                Some("input") => {
                    let text = m.get_one::<String>("text").cloned().unwrap_or_default();
                    state.set_input(text);
                    true
                }
                _ => !state.open,
            };
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("clear").about("Clear the console output"),
        |world, _| {
            world.resource_mut::<ConsoleState>().log.clear();
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("echo")
            .about("Print text")
            .arg(Arg::new("text").num_args(0..)),
        |_, m| {
            Ok(m.get_many::<String>("text")
                .map(|v| v.cloned().collect::<Vec<_>>().join(" "))
                .unwrap_or_default())
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("exec")
            .about("Run the commands in a file, one per line")
            .arg(Arg::new("file").required(true)),
        |world, m| {
            let path = m.get_one::<String>("file").expect("required");
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let mut state = world.resource_mut::<ConsoleState>();
            // Run the file's commands next, before anything queued after `exec`.
            for line in text.lines().rev() {
                state.pending.push_front(line.to_owned());
            }
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("fps")
            .about("Show or hide the FPS counter")
            .arg(Arg::new("on").value_parser(["on", "off"])),
        |world, m| {
            let mut show = world.resource_mut::<ShowFps>();
            show.0 = match m.get_one::<String>("on").map(String::as_str) {
                Some("on") => true,
                Some(_) => false,
                None => !show.0,
            };
            Ok(format!("fps counter {}", if show.0 { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("hud")
            .about("Show or hide the HUD")
            .arg(Arg::new("on").value_parser(["on", "off"])),
        |world, m| {
            let mut show = world.resource_mut::<ShowHud>();
            show.0 = match m.get_one::<String>("on").map(String::as_str) {
                Some("on") => true,
                Some(_) => false,
                None => !show.0,
            };
            Ok(format!("HUD {}", if show.0 { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("hud.units")
            .about("Units of speed and distance in the HUD")
            .arg(
                Arg::new("units")
                    .required(true)
                    .value_parser(["metric", "imperial"]),
            ),
        |world, m| {
            let units: Units = m.get_one::<String>("units").expect("required").parse()?;
            *world.resource_mut::<Units>() = units;
            Ok(format!("HUD units {units:?}"))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("hud.profile")
            .about("Show or hide the elevation profile of the road ahead")
            .arg(Arg::new("on").value_parser(["on", "off"])),
        |world, m| {
            let mut show = world.resource_mut::<ShowProfile>();
            show.0 = match m.get_one::<String>("on").map(String::as_str) {
                Some("on") => true,
                Some(_) => false,
                None => !show.0,
            };
            Ok(format!(
                "elevation profile {}",
                if show.0 { "on" } else { "off" }
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("hud.gps")
            .about("Show or hide the GPS line (bottom left: latitude, longitude, altitude)")
            .arg(Arg::new("on").value_parser(["on", "off"])),
        |world, m| {
            let mut show = world.resource_mut::<ShowGps>();
            show.0 = match m.get_one::<String>("on").map(String::as_str) {
                Some("on") => true,
                Some(_) => false,
                None => !show.0,
            };
            Ok(format!("GPS line {}", if show.0 { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("rider.gps")
            .about("Show the rider's GPS position (lat, lon, altitude; the HUD's GPS line)"),
        |world, _| match world
            .get_resource::<crate::RideView>()
            .and_then(|v| v.position)
        {
            Some((lat, lon, alt)) => Ok(format!(
                "{}\n{lat:.6}, {lon:.6}",
                crate::hud::gps_line(lat, lon, alt)
            )),
            None => Err("no GPS position (this route has no geographic origin)".into()),
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("camera.tilt")
            .about("Tilt the chase camera with the road: on, off, or the fraction of the slope angle (0-1)")
            .arg(Arg::new("value")),
        |world, m| {
            let mut tilt = world.resource_mut::<CameraTilt>();
            match m.get_one::<String>("value").map(String::as_str) {
                None => tilt.enabled = !tilt.enabled,
                Some("on") => tilt.enabled = true,
                Some("off") => tilt.enabled = false,
                Some(v) => {
                    let f: f32 = v
                        .parse()
                        .map_err(|_| format!("'{v}': expected on, off or 0-1"))?;
                    if !(0.0..=1.0).contains(&f) {
                        return Err(format!("'{v}': the fraction is 0-1"));
                    }
                    tilt.fraction = f;
                    tilt.enabled = f > 0.0;
                }
            }
            Ok(if tilt.enabled {
                format!(
                    "camera tilt on ({:.0} % of the slope, at most {:.0} degrees)",
                    tilt.fraction * 100.0,
                    tilt.max.to_degrees()
                )
            } else {
                "camera tilt off".into()
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("camera.mode")
            .about("Set the camera perspective")
            .arg(
                Arg::new("mode")
                    .required(true)
                    .value_parser(["chase", "side", "top"]),
            ),
        |world, m| {
            let mode: CameraMode = m.get_one::<String>("mode").expect("required").parse()?;
            *world.resource_mut::<CameraMode>() = mode;
            Ok(format!("camera mode {mode:?}"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("camera.drone")
            .about("Dev (videos, spec §2.11): the camera on the ride photographer's drone while it flies – pov (its camera's view), follow (behind and aside of it) or off")
            .arg(Arg::new("mode").value_parser(["pov", "follow", "off"])),
        |world, m| {
            use crate::drone_camera::{DroneCamera, drone_of};
            if let Some(v) = m.get_one::<String>("mode") {
                *world.resource_mut::<DroneCamera>() = v.parse()?;
            }
            let mode = *world.resource::<DroneCamera>();
            let flying = world
                .get_resource::<crate::RideView>()
                .and_then(drone_of)
                .is_some();
            Ok(format!(
                "drone camera {}{}",
                mode.label(),
                match (mode, flying) {
                    (DroneCamera::Off, _) => "",
                    (_, true) => " (a drone is out)",
                    (_, false) => " (waits for a drone: `photographer drone`)",
                }
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("camera.orbit")
            .about("Orbit the chase camera: yaw (0 = behind, 180 = in front) and pitch in degrees, distance in m")
            .arg(Arg::new("yaw").required(true).allow_negative_numbers(true).value_parser(value_parser!(f32)))
            .arg(Arg::new("pitch").required(true).allow_negative_numbers(true).value_parser(value_parser!(f32)))
            .arg(Arg::new("distance").value_parser(value_parser!(f32))),
        |world, m| {
            let mut orbit = world.resource_mut::<CameraOrbit>();
            orbit.yaw = m.get_one::<f32>("yaw").expect("required").to_radians();
            orbit.pitch = m
                .get_one::<f32>("pitch")
                .expect("required")
                .to_radians()
                .clamp(MIN_PITCH, MAX_PITCH);
            if let Some(d) = m.get_one::<f32>("distance") {
                orbit.distance = d.clamp(MIN_DISTANCE, MAX_DISTANCE);
            }
            Ok(format!(
                "camera orbit yaw {:.0}, pitch {:.0}, distance {:.1} m",
                orbit.yaw.to_degrees(),
                orbit.pitch.to_degrees(),
                orbit.distance
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("camera.look")
            .about("Look at the sky from just behind the rider: azimuth (0 = north, 90 = east) and elevation in degrees; 'camera.look off' returns to the chase camera")
            .arg(Arg::new("azimuth").required(true).allow_negative_numbers(true))
            .arg(Arg::new("elevation").allow_negative_numbers(true).value_parser(value_parser!(f32))),
        |world, m| {
            let az = m.get_one::<String>("azimuth").expect("required");
            let mut look = world.resource_mut::<crate::camera::SkyLook>();
            if az == "off" {
                look.0 = None;
                return Ok("camera: chase".into());
            }
            let az: f32 = az.parse().map_err(|_| format!("azimuth: not a number: {az}"))?;
            let el = m.get_one::<f32>("elevation").copied().unwrap_or(20.0).clamp(-10.0, 90.0);
            look.0 = Some((az.rem_euclid(360.0), el));
            Ok(format!("camera looking at azimuth {:.0}°, elevation {el:.0}°", az.rem_euclid(360.0)))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("screenshot")
            .about("Save a screenshot of the window (in a headset session: the left eye): JPEG with its time in EXIF (a .png path: PNG); in the browser a download named with the time")
            .arg(Arg::new("path")),
        |world, m| {
            let meta = crate::stamp::ShotMeta::of(world);
            // The browser app (spec §14 Phase 10 B6): a download.
            if let Some(sink) = world.get_resource::<crate::stamp::ShotSink>().cloned() {
                let name = m.get_one::<String>("path").cloned().unwrap_or_else(|| {
                    crate::stamp::capture_name(&meta.time, "jpg")
                });
                world
                    .commands()
                    .spawn(Screenshot::primary_window())
                    .observe(crate::stamp::sink_shot(name.clone(), meta, sink));
                return Ok(format!("screenshot: downloading {name}"));
            }
            let path = m
                .get_one::<String>("path")
                .cloned()
                .unwrap_or_else(|| "screenshot.jpg".into());
            // A headset session: the left eye, as `--screenshot` (the
            // window is the mirror, with the flat HUD over it).
            let headset = world
                .get_resource::<crate::vr::VrConfig>()
                .is_some_and(|c| c.mode == crate::vr::VrMode::Headset);
            let eye = world
                .get_resource::<crate::vr::EyeImages>()
                .map(|e| e.eyes[0].clone());
            let (source, what) = match (eye, crate::autoshot::shot_from(true, headset, true)) {
                (Some(eye), crate::autoshot::ShotFrom::LeftEye) if headset => {
                    (Screenshot::image(eye), "the left eye")
                }
                _ => (Screenshot::primary_window(), "the window"),
            };
            world
                .commands()
                .spawn(source)
                .observe(crate::stamp::save_shot(path.clone(), meta));
            Ok(format!("saving a screenshot of {what} to {path}"))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("quit").about("Exit the application"),
        |world, _| {
            world.write_message(AppExit::Success);
            Ok("bye".into())
        },
    ))
    // `help COMMAND`: the commands this session may run.
    .console_values("help", "command", |world, _| {
        let allowed = world.resource::<ConsoleState>().permission;
        world
            .resource::<ConsoleCommands>()
            .0
            .iter()
            .filter(|(_, c)| c.level <= allowed)
            .map(|(n, _)| n.clone())
            .collect()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_line_handles_quotes() {
        assert_eq!(
            split_line("  echo a  \"b c\" d"),
            vec!["echo", "a", "b c", "d"]
        );
        assert_eq!(split_line("echo \"\""), vec!["echo", ""]);
        assert!(split_line("   ").is_empty());
    }

    /// Regression (spec §14 Phase 4, 4M): while the workshop is open the
    /// console was drawn into the ride window (the world camera renders
    /// into an image then); it must follow the camera on screen.
    #[test]
    fn the_console_is_drawn_by_the_camera_on_screen() {
        let mut ids = World::new();
        let ids: Vec<Entity> = (0..4).map(|_| ids.spawn_empty().id()).collect();
        let e = |i: usize| ids[i];
        // The world camera alone; then the workshop's camera on top while
        // the world camera renders into the ride window (an image).
        assert_eq!(console_host([(e(1), 0, true, true)]), Some(e(1)));
        assert_eq!(
            console_host([
                (e(1), 0, true, false),
                (e(2), 1, true, true),
                (e(3), 0, true, false)
            ]),
            Some(e(2))
        );
        assert_eq!(console_host([(e(1), 0, false, true)]), None);
        assert_eq!(console_host([(e(1), 0, true, false)]), None);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(PostUpdate, console_on_top);
        let world_cam = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                EguiContext::default(),
                PrimaryEguiContext,
            ))
            .id();
        app.update();
        let has = |app: &App, cam| app.world().get::<PrimaryEguiContext>(cam).is_some();
        assert!(has(&app, world_cam));
        // The workshop opens: the world camera into the ride window, the
        // workshop camera (order 1) on screen.
        *app.world_mut().get_mut::<RenderTarget>(world_cam).unwrap() =
            RenderTarget::Image(Handle::<Image>::default().into());
        let workshop = app
            .world_mut()
            .spawn((Camera {
                order: 1,
                ..default()
            },))
            .id();
        app.update();
        assert!(has(&app, workshop), "the console is over the workshop");
        assert!(!has(&app, world_cam));
        let w = app.world();
        assert!(w.get::<EguiContext>(world_cam).is_none());
        assert!(w.get::<EguiSchedule>(world_cam).is_none(), "one pass only");
        // Its (empty) output stays: bevy_egui's render view of the camera.
        assert!(w.get::<bevy_egui::EguiRenderOutput>(world_cam).is_some());
        assert!(w.get::<EguiContext>(workshop).is_some());
        assert!(w.get::<EguiSchedule>(workshop).is_some());
        // Closed: the workshop camera goes, the world camera is on screen.
        app.world_mut().despawn(workshop);
        *app.world_mut().get_mut::<RenderTarget>(world_cam).unwrap() =
            RenderTarget::Window(WindowRef::Primary);
        app.update();
        assert!(has(&app, world_cam));
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<PrimaryEguiContext>>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn permissions_are_ordered() {
        assert!(Permission::User < Permission::Dev && Permission::Dev < Permission::Admin);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin));
        app.console_command(ConsoleCommand::new(
            Permission::Dev,
            Command::new("dev.thing"),
            |_, _| Ok("did it".into()),
        ));
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().log.clear();
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().to_vec()
    }

    #[test]
    fn runs_commands_and_checks_permissions() {
        let mut app = app();
        assert_eq!(
            run(&mut app, "echo hello world"),
            vec!["> echo hello world", "hello world"]
        );
        assert!(run(&mut app, "fps on")[1].contains("on"));
        assert!(app.world().resource::<ShowFps>().0);
        assert_eq!(run(&mut app, "hud off")[1], "HUD off");
        assert!(!app.world().resource::<ShowHud>().0);
        assert_eq!(run(&mut app, "hud")[1], "HUD on");
        assert_eq!(run(&mut app, "hud.units imperial")[1], "HUD units Imperial");
        assert_eq!(*app.world().resource::<Units>(), Units::Imperial);
        assert!(run(&mut app, "hud.units furlongs")[1].starts_with("error:"));
        assert!(run(&mut app, "nope")[1].starts_with("error: unknown command"));
        assert!(run(&mut app, "dev.thing")[1].contains("needs Dev permission"));
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        assert_eq!(run(&mut app, "dev.thing")[1], "did it");
        // clap validation errors are shown, not panicked on.
        assert!(run(&mut app, "fps maybe")[1].starts_with("error:"));
        // camera.mode parses and sets the resource; bad values are errors.
        assert!(run(&mut app, "camera.mode top")[1].contains("Top"));
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Top);
        assert!(run(&mut app, "camera.mode sideways")[1].starts_with("error:"));
        // camera.drone is a dev command (spec §2.11, videos).
        {
            use crate::drone_camera::DroneCamera;
            app.world_mut().resource_mut::<ConsoleState>().permission = Permission::User;
            assert!(run(&mut app, "camera.drone pov")[1].contains("needs Dev permission"));
            assert_eq!(*app.world().resource::<DroneCamera>(), DroneCamera::Off);
            app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
            assert!(run(&mut app, "camera.drone pov")[1].starts_with("drone camera pov"));
            assert_eq!(*app.world().resource::<DroneCamera>(), DroneCamera::Pov);
            assert!(run(&mut app, "camera.drone follow")[1].starts_with("drone camera follow"));
            assert_eq!(*app.world().resource::<DroneCamera>(), DroneCamera::Follow);
            assert!(run(&mut app, "camera.drone")[1].starts_with("drone camera follow"));
            assert!(run(&mut app, "camera.drone sideways")[1].starts_with("error:"));
            assert!(run(&mut app, "camera.drone off")[1].starts_with("drone camera off"));
            assert_eq!(*app.world().resource::<DroneCamera>(), DroneCamera::Off);
        }
        assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Top);
        // camera.orbit sets the orbit in degrees, clamped.
        assert!(run(&mut app, "camera.orbit 90 30 12")[1].starts_with("camera orbit yaw 90"));
        let o = *app.world().resource::<CameraOrbit>();
        assert!((o.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5 && o.distance == 12.0);
        run(&mut app, "camera.orbit -45 200");
        assert_eq!(app.world().resource::<CameraOrbit>().pitch, MAX_PITCH);
        // hud.profile toggles or sets the elevation profile.
        assert!(app.world().resource::<ShowProfile>().0, "on by default");
        assert!(run(&mut app, "hud.profile")[1].ends_with("off"));
        assert!(!app.world().resource::<ShowProfile>().0);
        assert!(run(&mut app, "hud.profile on")[1].ends_with("on"));
        assert!(app.world().resource::<ShowProfile>().0);
        assert!(run(&mut app, "hud.profile maybe")[1].starts_with("error:"));
        // camera.tilt: on/off, or the fraction of the slope angle.
        assert!(app.world().resource::<CameraTilt>().enabled);
        assert_eq!(run(&mut app, "camera.tilt off")[1], "camera tilt off");
        assert!(!app.world().resource::<CameraTilt>().enabled);
        assert!(run(&mut app, "camera.tilt 0.5")[1].starts_with("camera tilt on (50 %"));
        let t = *app.world().resource::<CameraTilt>();
        assert!(t.enabled && t.fraction == 0.5);
        assert!(run(&mut app, "camera.tilt 2")[1].starts_with("error:"));
        assert!(run(&mut app, "camera.tilt steep")[1].starts_with("error:"));
        assert_eq!(app.world().resource::<CameraTilt>().fraction, 0.5);
        run(&mut app, "camera.tilt");
        assert!(!app.world().resource::<CameraTilt>().enabled);
        // help lists commands.
        assert!(run(&mut app, "help").iter().any(|l| l.starts_with("echo")));
    }

    fn tab(app: &mut App, line: &str) -> (String, Vec<String>) {
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .set_input(line);
        tab_complete(app.world_mut(), line.len(), false);
        let s = app.world().resource::<ConsoleState>();
        (s.input().to_owned(), s.candidates().0.to_vec())
    }

    /// Tab in the console: names, values from the clap definition and
    /// from a provider, and never a command the session may not run
    /// (spec §2.9: no dev commands on a remote server).
    #[test]
    fn tab_completes_by_permission_and_providers() {
        let mut app = app();
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("pick").arg(Arg::new("thing")),
            |_, _| Ok(String::new()),
        ))
        .console_values("pick", "thing", |_, q| {
            assert_eq!(q.words, ["pick"]);
            vec!["apple".into(), "apricot".into(), "banana".into()]
        });
        assert_eq!(tab(&mut app, "hud.u").0, "hud.units ");
        assert_eq!(tab(&mut app, "hud.units i").0, "hud.units imperial ");
        assert_eq!(tab(&mut app, "pick b").0, "pick banana ");
        let (line, listed) = tab(&mut app, "pick a");
        assert_eq!((line.as_str(), listed.len()), ("pick ap", 2));
        // The next Tab inserts the first of the list.
        tab_complete(app.world_mut(), line.len(), false);
        assert_eq!(
            app.world().resource::<ConsoleState>().input(),
            "pick apple "
        );
        assert_eq!(
            app.world().resource::<ConsoleState>().candidates().1,
            Some(0)
        );
        // Dev commands: only where they can run.
        assert_eq!(tab(&mut app, "dev.").0, "dev.");
        assert!(tab(&mut app, "help dev.").1.is_empty());
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        assert_eq!(tab(&mut app, "dev.").0, "dev.thing ");
        assert_eq!(tab(&mut app, "help dev.t").0, "help dev.thing ");
        // Typing ends the list.
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .set_input("x");
        assert!(
            app.world()
                .resource::<ConsoleState>()
                .candidates()
                .0
                .is_empty()
        );
        // The overlay's Tab is run the next frame; `console input` / `tab`
        // do the same from scripts.
        run(&mut app, "console input \"hud.p\"");
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .request_tab(None, false);
        app.update();
        assert_eq!(
            app.world().resource::<ConsoleState>().input(),
            "hud.profile "
        );
        run(&mut app, "console input hud");
        run(&mut app, "console tab");
        let s = app.world().resource::<ConsoleState>();
        assert!(s.is_open());
        assert_eq!(s.input(), "hud");
        assert_eq!(
            s.candidates().0,
            ["hud", "hud.gps", "hud.profile", "hud.units"]
        );
    }

    /// The GPS line's commands (shared by both apps since spec 3.40):
    /// `hud.gps` switches the line, `rider.gps` prints the position.
    #[test]
    fn gps_commands() {
        let mut app = app();
        assert!(app.world().resource::<ShowGps>().0, "on by default");
        assert_eq!(run(&mut app, "hud.gps off")[1], "GPS line off");
        assert!(!app.world().resource::<ShowGps>().0);
        assert_eq!(run(&mut app, "hud.gps")[1], "GPS line on");
        assert!(run(&mut app, "rider.gps")[1].starts_with("error: no GPS position"));
        let mut view = crate::RideView::new(ocw_world::Route::test_loop());
        view.position = Some((47.139961, 9.521512, 455.2));
        app.insert_resource(view);
        let out = run(&mut app, "rider.gps");
        assert_eq!(out[1], "47.13996° N  9.52151° E  455 m");
        assert_eq!(out[2], "47.139961, 9.521512");
    }
}
