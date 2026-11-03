//! The engine's input queue: what the platform delivered between two frames, handed to the frame that follows.

use metallic_platform::{DeviceEvent, GamepadEvent, WindowEvent, WindowId};

/// One input event, as the platform delivered it.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// Keyboard, mouse, touch or focus input on a window.
    Window {
        /// The window it happened on.
        window: WindowId,
        /// What happened.
        event: WindowEvent,
    },
    /// Raw device input, not tied to a window (mouse motion for cameras).
    Device(DeviceEvent),
    /// A gamepad connected, disconnected or changed.
    Gamepad(GamepadEvent),
}

/// Events queued since the last frame, and the ones the current frame took.
///
/// Two buffers swapped at the start of each frame, so after warm-up queuing and taking allocate nothing.
#[derive(Debug, Default)]
pub(crate) struct InputQueue {
    /// Delivered since the current frame started.
    pending: Vec<InputEvent>,
    /// What the current frame took.
    frame: Vec<InputEvent>,
}

impl InputQueue {
    /// Queues an event for the next frame.
    pub(crate) fn push(&mut self, event: InputEvent) {
        self.pending.push(event);
    }

    /// The frame's input step: the events queued so far become the frame's.
    pub(crate) fn begin_frame(&mut self) {
        self.frame.clear();
        std::mem::swap(&mut self.pending, &mut self.frame);
    }

    /// The events the current frame took, in the order they arrived.
    pub(crate) fn frame_events(&self) -> &[InputEvent] {
        &self.frame
    }

    /// Events waiting for the next frame.
    #[cfg(test)]
    fn pending(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use metallic_platform::WindowEvent;

    use super::*;

    fn focus(focused: bool) -> InputEvent {
        InputEvent::Window {
            window: WindowId::from_raw(1),
            event: WindowEvent::Focused(focused),
        }
    }

    #[test]
    fn each_frame_takes_what_arrived_before_it() {
        let mut queue = InputQueue::default();
        queue.push(focus(true));
        queue.push(focus(false));
        assert!(queue.frame_events().is_empty());
        queue.begin_frame();
        assert_eq!(queue.frame_events(), [focus(true), focus(false)]);
        assert_eq!(queue.pending(), 0);
        queue.push(focus(true));
        queue.begin_frame();
        assert_eq!(queue.frame_events(), [focus(true)]);
        queue.begin_frame();
        assert!(queue.frame_events().is_empty());
    }
}
