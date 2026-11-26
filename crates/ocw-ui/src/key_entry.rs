//! A secret typed into a menu row: the ANT+ network key on Devices → ANT+
//! (spec 3.4.1, 14 Phase 17 Q1 (b)). While typing, this frame's keys are
//! the field's (as for Ride → Join's fields: [`crate::join::edit_field`]);
//! the row shows the field masked – one `*` per character, the cursor and
//! the count of hex digits – never the key. Enter or Esc ends; the app
//! reads how ([`KeyEntry::take_end`]) and saves or drops the text.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use ocw_client::menu::join::TextField;

use crate::join::{FieldEnd, ctrl_held, edit_field, pads_pressed};

/// The field and whether it is being typed into.
#[derive(Resource, Default)]
pub struct KeyEntry {
    pub field: TextField,
    pub typing: bool,
    end: Option<FieldEnd>,
    paste: bool,
}

impl KeyEntry {
    /// Starts typing into an empty field.
    pub fn start(&mut self) {
        self.field.clear();
        self.typing = true;
        self.end = None;
        self.paste = false;
    }

    /// Stops typing; the text is dropped.
    pub fn cancel(&mut self) {
        self.field.clear();
        self.typing = false;
        self.end = None;
    }

    /// How the typing ended since the last call (Enter: save; Esc: drop).
    pub fn take_end(&mut self) -> Option<FieldEnd> {
        self.end.take()
    }

    /// Ctrl+V while typing (the app pastes its clipboard).
    pub fn take_paste(&mut self) -> bool {
        std::mem::take(&mut self.paste)
    }

    /// The text (for the app to parse; never for a log).
    pub fn text(&self) -> &str {
        &self.field.text
    }

    /// The row's value: `********|  8 of 16`.
    pub fn shown(&self, digits_wanted: usize) -> String {
        let n = self.field.text.chars().count();
        let cursor = self.field.cursor.min(n);
        let mut s = "*".repeat(cursor);
        s.push('|');
        s.push_str(&"*".repeat(n - cursor));
        let digits = self
            .field
            .text
            .chars()
            .filter(char::is_ascii_hexdigit)
            .count();
        format!("{s}  {digits} of {digits_wanted}")
    }
}

/// Where [`capture`] has read the typed keys up to.
#[derive(Resource, Default)]
struct KeyEntryCursor(bevy::ecs::message::MessageCursor<KeyboardInput>);

/// While the key is typed: this frame's input is the field's. `true`: the
/// input was taken.
pub fn capture(world: &mut World) -> bool {
    world.init_resource::<KeyEntryCursor>();
    let typed: Vec<KeyboardInput> = world.resource_scope(|world, mut c: Mut<KeyEntryCursor>| {
        world
            .get_resource::<Messages<KeyboardInput>>()
            .map(|m| c.0.read(m).cloned().collect())
            .unwrap_or_default()
    });
    if !world.get_resource::<KeyEntry>().is_some_and(|k| k.typing) {
        return false;
    }
    let pads = pads_pressed(world);
    let ctrl = ctrl_held(world);
    let mut entry = world.resource_mut::<KeyEntry>();
    let (paste, end) = edit_field(&mut entry.field, &typed, &pads, ctrl);
    if paste {
        entry.paste = true;
    }
    if let Some(e) = end {
        entry.typing = false;
        entry.end = Some(e);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::ButtonState;
    use bevy::input::keyboard::Key;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<KeyboardInput>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<KeyEntry>();
        app.update();
        app
    }

    fn key(app: &mut App, key: Key, text: Option<&str>) {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyA,
            logical_key: key,
            state: ButtonState::Pressed,
            text: text.map(Into::into),
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        capture(app.world_mut());
    }

    #[test]
    fn the_key_is_typed_masked_and_ended_with_enter_or_esc() {
        let mut app = app();
        assert!(!capture(app.world_mut()), "not typing");
        app.world_mut().resource_mut::<KeyEntry>().start();
        for c in "0a1b".chars() {
            let t = c.to_string();
            key(&mut app, Key::Character(t.clone().into()), Some(&t));
        }
        key(&mut app, Key::Backspace, None);
        let e = app.world().resource::<KeyEntry>();
        assert_eq!(e.text(), "0a1");
        assert_eq!(e.shown(16), "***|  3 of 16");
        assert!(e.typing);
        key(&mut app, Key::ArrowLeft, None);
        assert_eq!(
            app.world().resource::<KeyEntry>().shown(16),
            "**|*  3 of 16"
        );
        key(&mut app, Key::Enter, None);
        let mut e = app.world_mut().resource_mut::<KeyEntry>();
        assert!(!e.typing);
        assert_eq!(e.take_end(), Some(FieldEnd::Enter));
        assert_eq!(e.take_end(), None);
        // Esc cancels.
        e.start();
        key(&mut app, Key::Character("f".into()), Some("f"));
        key(&mut app, Key::Escape, None);
        let mut e = app.world_mut().resource_mut::<KeyEntry>();
        assert_eq!(e.take_end(), Some(FieldEnd::Escape));
        e.cancel();
        assert_eq!(e.text(), "");
    }
}
