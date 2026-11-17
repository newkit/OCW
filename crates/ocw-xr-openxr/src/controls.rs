//! The controllers' actions (spec §14 Phase 11, v4.38): one OpenXR action
//! set with an action per `ocw_xr::controllers::Input`, the suggested
//! bindings of `ocw_xr::controllers::BINDINGS` (Oculus Touch, the Khronos
//! simple controller), attached to each session and synced every frame.
//!
//! The first headset run (2026-11-17, Quest 1 over Link) had no action set
//! at all – X1 had left the controllers out (Q7) – so the runtime routed
//! nothing to the app: without `xrCreateActionSet`,
//! `xrSuggestInteractionProfileBindings`, `xrAttachSessionActionSets` and
//! `xrSyncActions` every frame, an OpenXR app sees no button.

use ocw_xr::controllers::{
    self, ACTION_SET, ACTION_SET_LOCALIZED, ControllerState, Input, InputKind, InputValue,
};
use ocw_xr::session::XrError;
use openxr as xr;

use crate::xr_error;

enum Action {
    Button(xr::Action<bool>),
    Value(xr::Action<f32>),
    Stick(xr::Action<xr::Vector2f>),
}

/// The action set and its actions (the instance's; every session attaches
/// it).
pub struct Controls {
    // Actions before their set (drop order).
    actions: Vec<(Input, Action)>,
    set: xr::ActionSet,
}

impl Controls {
    /// Creates the set, its actions and the suggested bindings. A profile
    /// the runtime does not know is skipped (with a warning); no profile
    /// at all is an error.
    pub fn new(instance: &xr::Instance) -> Result<Self, XrError> {
        let set = instance
            .create_action_set(ACTION_SET, ACTION_SET_LOCALIZED, 0)
            .map_err(xr_error)?;
        let mut actions = Vec::new();
        for i in Input::ALL {
            let a = match i.kind() {
                InputKind::Button => Action::Button(
                    set.create_action(i.name(), i.localized(), &[])
                        .map_err(xr_error)?,
                ),
                InputKind::Value => Action::Value(
                    set.create_action(i.name(), i.localized(), &[])
                        .map_err(xr_error)?,
                ),
                InputKind::Stick => Action::Stick(
                    set.create_action(i.name(), i.localized(), &[])
                        .map_err(xr_error)?,
                ),
            };
            actions.push((i, a));
        }
        let mut suggested = 0;
        for profile in controllers::profiles() {
            match suggest(instance, profile, &actions) {
                Ok(()) => suggested += 1,
                Err(e) => tracing::warn!("VR: bindings for {profile} refused: {e}"),
            }
        }
        if suggested == 0 {
            return Err(XrError::Unsupported(
                "the runtime takes no controller bindings".into(),
            ));
        }
        Ok(Self { actions, set })
    }

    /// `xrAttachSessionActionSets` (once per session).
    pub fn attach(&self, session: &xr::Session<xr::Vulkan>) -> Result<(), XrError> {
        session.attach_action_sets(&[&self.set]).map_err(xr_error)
    }

    /// `xrSyncActions` and every action's state. Outside the input focus
    /// the runtime reports the actions inactive (`XR_SESSION_NOT_FOCUSED`
    /// is a success code): the state is then `active: false`.
    pub fn sync(&self, session: &xr::Session<xr::Vulkan>) -> Result<ControllerState, XrError> {
        session
            .sync_actions(&[xr::ActiveActionSet::new(&self.set)])
            .map_err(xr_error)?;
        let mut out = ControllerState::default();
        let none = xr::Path::NULL;
        for (i, a) in &self.actions {
            let (value, active) = match a {
                Action::Button(a) => {
                    let s = a.state(session, none).map_err(xr_error)?;
                    (InputValue::Button(s.current_state), s.is_active)
                }
                Action::Value(a) => {
                    let s = a.state(session, none).map_err(xr_error)?;
                    (InputValue::Value(s.current_state), s.is_active)
                }
                Action::Stick(a) => {
                    let s = a.state(session, none).map_err(xr_error)?;
                    (
                        InputValue::Stick([s.current_state.x, s.current_state.y]),
                        s.is_active,
                    )
                }
            };
            if active {
                out.set(*i, value);
                out.active = true;
            }
        }
        Ok(out)
    }

    /// The profile the runtime chose per hand (left, right).
    pub fn profiles(
        &self,
        instance: &xr::Instance,
        session: &xr::Session<xr::Vulkan>,
    ) -> [String; 2] {
        ["/user/hand/left", "/user/hand/right"].map(|hand| {
            instance
                .string_to_path(hand)
                .and_then(|p| session.current_interaction_profile(p))
                .ok()
                .filter(|p| *p != xr::Path::NULL)
                .and_then(|p| instance.path_to_string(p).ok())
                .unwrap_or_default()
        })
    }
}

fn suggest(
    instance: &xr::Instance,
    profile: &str,
    actions: &[(Input, Action)],
) -> Result<(), XrError> {
    let mut paths = Vec::new();
    for b in controllers::bindings_of(profile) {
        paths.push((b.input, instance.string_to_path(b.path).map_err(xr_error)?));
    }
    let bindings: Vec<xr::Binding<'_>> = paths
        .iter()
        .filter_map(|(input, path)| {
            let (_, a) = actions.iter().find(|(i, _)| i == input)?;
            Some(match a {
                Action::Button(a) => xr::Binding::new(a, *path),
                Action::Value(a) => xr::Binding::new(a, *path),
                Action::Stick(a) => xr::Binding::new(a, *path),
            })
        })
        .collect();
    let profile = instance.string_to_path(profile).map_err(xr_error)?;
    instance
        .suggest_interaction_profile_bindings(profile, &bindings)
        .map_err(xr_error)
}
