//! Per-game state of class code (#810): what a service keeps between calls that is not instances or properties,
//! such as `TweenService`'s playing tweens. One value of each type per data model: class code takes it out
//! ([`DataModel::take_state`]), works on it and the data model together, and puts it back.

use std::any::{Any, TypeId};
use std::fmt;

use super::DataModel;

/// The values, by type. A handful at most, so a list: a frame that asks whether one exists only compares ids.
#[derive(Default)]
pub(super) struct States(Vec<(TypeId, Box<dyn Any + Send>)>);

impl fmt::Debug for States {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "States({} kinds)", self.0.len())
    }
}

impl DataModel {
    /// Whether class code stored any state: the engine asks every frame before stepping what keeps some (tweens),
    /// so a game with none pays nothing more.
    #[inline]
    pub fn has_states(&self) -> bool {
        !self.states.0.is_empty()
    }

    /// The game's `T`, if it was stored.
    pub fn state<T: Any + Send>(&self) -> Option<&T> {
        let id = TypeId::of::<T>();
        self.states.0.iter().find(|(t, _)| *t == id)?.1.downcast_ref::<T>()
    }

    /// Takes the game's `T` out, leaving none: to work on it while also changing the data model, then put it back
    /// with [`DataModel::put_state`].
    pub fn take_state<T: Any + Send>(&mut self) -> Option<T> {
        let id = TypeId::of::<T>();
        let at = self.states.0.iter().position(|(t, _)| *t == id)?;
        let (_, boxed) = self.states.0.swap_remove(at);
        boxed.downcast::<T>().ok().map(|state| *state)
    }

    /// Stores the game's `T`, replacing any.
    pub fn put_state<T: Any + Send>(&mut self, state: T) {
        let id = TypeId::of::<T>();
        self.states.0.retain(|(t, _)| *t != id);
        self.states.0.push((id, Box::new(state)));
    }
}
