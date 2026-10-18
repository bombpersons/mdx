//! Triggers and actions: level data says *when* (an area, an action id and
//! its arguments); the game registers *what* (one handler per action id).
//!
//! Handlers are one-shot systems taking the [`ActionCall`]. Unknown action
//! ids are logged and counted in [`UnknownActions`], never ignored or mapped
//! to something else.
//!
//! [`TriggerArea`] is a placeholder detector: a rectangle that fires when a
//! [`TriggerActivator`] enters it. Replace it with whatever the original's
//! trigger model is (event tables keyed by camera position, special tiles,
//! objects that act as triggers); keep the action registry.
use crate::objects::Position;
use crate::schedule::EngineSet;
use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An action id, as level data stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActionId(pub u32);

/// One request to run an action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionCall {
    pub action: ActionId,
    /// Arguments whose meaning belongs to the action.
    #[serde(default)]
    pub args: Vec<i32>,
    /// The entity that caused it (the trigger area), if any.
    #[serde(skip)]
    pub source: Option<Entity>,
}

/// Write this to run an action during [`EngineSet::Triggers`].
#[derive(Message, Clone, Debug)]
pub struct FireAction(pub ActionCall);

/// A registered action handler.
#[derive(Clone)]
pub struct ActionInfo {
    pub name: &'static str,
    pub handler: SystemId<In<ActionCall>>,
}

/// Every registered action, by id.
#[derive(Resource, Default)]
pub struct Actions(pub BTreeMap<ActionId, ActionInfo>);

/// Calls whose action id nothing registered, with how many were seen.
#[derive(Resource, Default, Debug)]
pub struct UnknownActions(pub BTreeMap<u32, u32>);

pub trait ActionAppExt {
    /// Registers `handler` for `id`. Panics if `id` is already registered.
    fn register_action<M>(
        &mut self,
        id: ActionId,
        name: &'static str,
        handler: impl IntoSystem<In<ActionCall>, (), M> + 'static,
    ) -> &mut Self;
}

impl ActionAppExt for App {
    fn register_action<M>(
        &mut self,
        id: ActionId,
        name: &'static str,
        handler: impl IntoSystem<In<ActionCall>, (), M> + 'static,
    ) -> &mut Self {
        let handler = self.world_mut().register_system(handler);
        let mut actions = self.world_mut().get_resource_or_init::<Actions>();
        if let Some(old) = actions.0.insert(id, ActionInfo { name, handler }) {
            panic!("action {} registered twice (first as {:?})", id.0, old.name);
        }
        self
    }
}

/// Fires its call when a [`TriggerActivator`]'s position enters the
/// rectangle from the area's [`Position`] (top-left) to `Position + size`.
#[derive(Component, Clone, Debug)]
#[require(Position)]
pub struct TriggerArea {
    pub size: IVec2,
    pub call: ActionCall,
    /// Fire only the first time.
    pub once: bool,
    /// Runtime state: an activator was inside last tick / it has fired.
    pub inside: bool,
    pub fired: bool,
}

impl TriggerArea {
    pub fn new(size: IVec2, call: ActionCall, once: bool) -> Self {
        Self { size, call, once, inside: false, fired: false }
    }
}

/// Entities that set off trigger areas (usually the player).
#[derive(Component, Default, Clone, Copy, Debug)]
#[require(Position)]
pub struct TriggerActivator;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TriggerSystems {
    Detect,
    Dispatch,
}

pub struct TriggersPlugin;

impl Plugin for TriggersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Actions>()
            .init_resource::<UnknownActions>()
            .add_message::<FireAction>()
            .configure_sets(
                FixedUpdate,
                (TriggerSystems::Detect, TriggerSystems::Dispatch).chain().in_set(EngineSet::Triggers),
            )
            .add_systems(FixedUpdate, detect.in_set(TriggerSystems::Detect))
            .add_systems(FixedUpdate, dispatch.in_set(TriggerSystems::Dispatch));
    }
}

fn detect(
    mut areas: Query<(Entity, &Position, &mut TriggerArea)>,
    activators: Query<&Position, With<TriggerActivator>>,
    mut fire: MessageWriter<FireAction>,
) {
    for (entity, at, mut area) in &mut areas {
        let (min, max) = (at.0, at.0 + area.size);
        let inside = activators.iter().any(|a| a.0.cmpge(min).all() && a.0.cmplt(max).all());
        if inside && !area.inside && !(area.once && area.fired) {
            fire.write(FireAction(ActionCall { source: Some(entity), ..area.call.clone() }));
            area.fired = true;
        }
        area.inside = inside;
    }
}

/// Runs each fired action's handler, in the order they were fired.
fn dispatch(world: &mut World) {
    let calls: Vec<ActionCall> = world.resource_mut::<Messages<FireAction>>().drain().map(|m| m.0).collect();
    for call in calls {
        let handler = world.resource::<Actions>().0.get(&call.action).map(|a| a.handler);
        match handler {
            Some(handler) => {
                if let Err(e) = world.run_system_with(handler, call) {
                    error!("action handler failed: {e}");
                }
            }
            None => {
                warn!("no action {} registered; call {:?} ignored", call.action.0, call.args);
                *world.resource_mut::<UnknownActions>().0.entry(call.action.0).or_default() += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use md_bevy::shell::TickRate;
    use md_bevy::testing::{TickApp, headless_app};

    #[derive(Resource, Default)]
    struct Log(Vec<Vec<i32>>);

    #[test]
    fn entering_an_area_runs_its_action_once_per_entry() {
        let mut app = headless_app(TickRate::default());
        app.add_plugins(crate::EnginePlugin).init_resource::<Log>().register_action(
            ActionId(3),
            "log",
            |In(call): In<ActionCall>, mut log: ResMut<Log>| log.0.push(call.args),
        );
        let call = |action| ActionCall { action: ActionId(action), args: vec![action as i32], source: None };
        app.world_mut().spawn((Position(IVec2::ZERO), TriggerArea::new(IVec2::splat(16), call(3), false)));
        app.world_mut().spawn((Position(IVec2::new(64, 0)), TriggerArea::new(IVec2::splat(16), call(4), true)));
        let player = app.world_mut().spawn((Position(IVec2::new(8, 8)), TriggerActivator)).id();
        app.run_ticks(3, &[]);
        assert_eq!(app.world().resource::<Log>().0, vec![vec![3]]);
        app.world_mut().get_mut::<Position>(player).unwrap().0 = IVec2::new(70, 8);
        app.run_ticks(2, &[]);
        assert_eq!(app.world().resource::<UnknownActions>().0.get(&4), Some(&1));
        app.world_mut().get_mut::<Position>(player).unwrap().0 = IVec2::new(8, 8);
        app.run_ticks(1, &[]);
        assert_eq!(app.world().resource::<Log>().0, vec![vec![3], vec![3]]);
    }
}
