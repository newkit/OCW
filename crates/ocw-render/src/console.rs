//! In-game console (spec §2.9).
//!
//! Plugins register commands with [`ConsoleAppExt::console_command`]. Each
//! command has a clap argument definition, help text, a permission level and a
//! handler that gets exclusive access to the [`World`]. Commands can be typed in
//! the overlay (toggle with `` ` `` or F12), queued programmatically with
//! [`ConsoleState::queue`], or run from a file with `exec <file>`.

use std::collections::{BTreeMap, VecDeque};

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use clap::{Arg, ArgMatches, Command, value_parser};

use crate::camera::{
    CameraMode, CameraOrbit, CameraTilt, MAX_DISTANCE, MAX_PITCH, MIN_DISTANCE, MIN_PITCH,
};
use crate::hud::{ShowFps, ShowProfile};

/// Console with its egui overlay (toggle with `` ` `` or F12).
pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin::default());
        }
        app.add_plugins(ConsoleCorePlugin)
            .add_systems(Update, toggle_console.before(run_pending_commands))
            .add_systems(EguiPrimaryContextPass, console_ui);
    }
}

/// Command registry, state and execution without any UI or input handling.
/// Usable headless (tests, `--cmd` automation).
pub struct ConsoleCorePlugin;

impl Plugin for ConsoleCorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleCommands>()
            .init_resource::<ConsoleState>()
            .init_resource::<ShowFps>()
            .init_resource::<ShowProfile>()
            .init_resource::<CameraMode>()
            .init_resource::<CameraOrbit>()
            .init_resource::<CameraTilt>()
            .add_systems(Update, run_pending_commands);
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

pub trait ConsoleAppExt {
    fn console_command(&mut self, command: ConsoleCommand) -> &mut Self;
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

fn console_ui(mut contexts: EguiContexts, mut state: ResMut<ConsoleState>) -> Result {
    if !state.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let width = ctx.content_rect().width() - 24.0;
    let state = &mut *state;
    egui::Window::new("Console")
        .title_bar(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, [0.0, 8.0])
        .fixed_size([width, 300.0])
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
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .hint_text("command"),
            );
            response.request_focus();
            state.input.retain(|c| c != '`');
            let (enter, up, down) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                )
            });
            if enter && !state.input.trim().is_empty() {
                let line = std::mem::take(&mut state.input);
                state.history.push(line.clone());
                state.history_pos = None;
                state.pending.push_back(line);
            } else if up && !state.history.is_empty() {
                let pos = state
                    .history_pos
                    .map_or(state.history.len() - 1, |p| p.saturating_sub(1));
                state.history_pos = Some(pos);
                state.input = state.history[pos].clone();
            } else if down && let Some(p) = state.history_pos {
                if p + 1 < state.history.len() {
                    state.history_pos = Some(p + 1);
                    state.input = state.history[p + 1].clone();
                } else {
                    state.history_pos = None;
                    state.input.clear();
                }
            }
        });
    Ok(())
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
            .about("Open, close or toggle the console overlay")
            .arg(
                Arg::new("action")
                    .value_parser(["open", "close", "toggle"])
                    .default_value("toggle"),
            ),
        |world, m| {
            let mut state = world.resource_mut::<ConsoleState>();
            state.open = match m.get_one::<String>("action").map(String::as_str) {
                Some("open") => true,
                Some("close") => false,
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
        Command::new("screenshot")
            .about("Save a screenshot of the window as PNG")
            .arg(Arg::new("path").default_value("screenshot.png")),
        |world, m| {
            let path = m.get_one::<String>("path").expect("has default").clone();
            world
                .commands()
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));
            Ok(format!("saving screenshot to {path}"))
        },
    ))
    .console_command(ConsoleCommand::new(
        User,
        Command::new("quit").about("Exit the application"),
        |world, _| {
            world.write_message(AppExit::Success);
            Ok("bye".into())
        },
    ));
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
}
