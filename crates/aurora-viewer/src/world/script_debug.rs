//! LLFloaterIMNearbyChatHandler::processChat / LLFloaterScriptDebug::addScriptLine
//! (Firestorm, LGPL 2.1). Muting and bridge filtering happen before this routing.
//!
//! Every script error of our own objects is kept here, for the Script debug
//! window, and every script line an object sends, for the Console tab of the
//! script window that object is open in: a window's console says what its
//! object answered even when the chat does not show it, whoever owns the
//! object, while the Debug window keeps the Legacy Viewer's rule (our
//! objects alone, EXT-7762). What an object says with `llOwnerSay` is kept
//! for the console whether or not the debug window routes it. With "Show
//! script errors in the nearby chat" (ShowScriptErrors-Location 0, on by
//! default in Aurora, #1263) an error also goes on to the nearby chat and
//! the window does not open by itself; otherwise the window opens on it, as
//! with the Legacy Viewer's default. "Also send them on a channel" (off) queues each
//! one for that chat channel, a few per second.

use super::ChatLine;
use aurora_net::{ChatMessage, ChatSourceType, ChatType};
use std::{
    collections::VecDeque,
    time::{Instant, SystemTime},
};
use uuid::Uuid;

/// Script errors that can be sent on the channel in a burst...
const CHANNEL_BURST: f32 = 5.0;
/// ...then one per second: a looping error must not flood the channel.
const CHANNEL_PER_SECOND: f32 = 1.0;

/// How many lines each store keeps: a chatty script cannot grow one
/// without end.
const MOST: usize = 500;

#[derive(Default)]
pub struct Messages {
    /// The Debug window's lines: our objects' script errors, and the
    /// `llOwnerSay` of ours when the window is asked to route it.
    pub lines: VecDeque<ChatLine>,
    /// Every script line an object of the region sends, for the Console tab
    /// of the script window that object is open in: the window says what its
    /// object answered, whichever account owns it (the Debug window keeps the
    /// Legacy Viewer's rule, EXT-7762, for its own list).
    pub console: VecDeque<ChatLine>,
    /// Bumped by a line that opens the window.
    pub opens: u64,
    pub owner_say: bool,
    /// Script errors also go to the nearby chat (Preferences › Chat).
    pub in_chat: bool,
    /// The channel the script errors are also said on, when chosen.
    pub channel: Option<i32>,
    /// Lines waiting to be said on `channel`, taken by the app.
    pub outgoing: Vec<(i32, String)>,
    /// What may still be said on the channel (`CHANNEL_BURST` at most),
    /// counted at `refilled`.
    tokens: f32,
    refilled: Option<Instant>,
}

impl Messages {
    /// Keeps what an object of ours says on the debug channel, and what it
    /// says to its owner, and answers whether the line belongs to script
    /// debug alone -- kept out of the nearby chat. An object's `llOwnerSay` is
    /// kept even when the debug window does not route it, so that the Console
    /// of its script window lists it; the window itself shows it only when
    /// routed.
    pub fn route(&mut self, message: &ChatMessage, agent: Uuid, now: Instant) -> bool {
        if message.source_type != ChatSourceType::Object {
            return false;
        }
        let error = message.chat_type == ChatType::Debug;
        let owner_say = message.chat_type == ChatType::OwnerSay;
        if !error && !owner_say {
            return false;
        }
        let line = ChatLine {
            time: SystemTime::now(),
            from: message.from_name.clone(),
            text: message.message.clone(),
            kind: super::ChatKind::Object(message.chat_type),
            source: message.source_id,
            origin: super::Origin::Live,
            blocked: false,
            beyond_range: false,
            speaker: Default::default(),
            private_history: false,
        };
        // a script window's console: the object it shows, whoever owns it
        self.keep_console(line.clone());
        // EXT-7762: errors from another resident's objects are not ours.
        if message.owner_id != agent || agent.is_nil() {
            return !owner_say || self.owner_say;
        }
        self.keep(line);
        if error {
            if let Some(channel) = self.channel {
                self.queue_for_channel(channel, &message.from_name, &message.message, now);
            }
            // shown in the nearby chat too when asked to, and then it leaves
            // the window alone
            if self.in_chat {
                return false;
            }
        } else if !self.owner_say {
            // an owner-say the window is not asked to route stays in the chat
            return false;
        }
        // a line sent here shows nowhere else: it opens the window
        self.opens = self.opens.wrapping_add(1);
        true
    }

    /// Adds a line for the script windows' consoles, dropping the oldest.
    fn keep_console(&mut self, line: ChatLine) {
        if self.console.len() == MOST {
            self.console.pop_front();
        }
        self.console.push_back(line);
    }

    /// Adds a line, dropping the oldest one of its kind first: a script that
    /// says a lot to its owner must not push the errors the window is for out
    /// of its 500 lines.
    fn keep(&mut self, line: ChatLine) {
        if self.lines.len() == MOST {
            let error = matches!(line.kind, super::ChatKind::Object(ChatType::Debug));
            let oldest = self
                .lines
                .iter()
                .position(|kept| matches!(kept.kind, super::ChatKind::Object(ChatType::Debug)) == error);
            match oldest {
                Some(at) => {
                    self.lines.remove(at);
                }
                None => {
                    self.lines.pop_front();
                }
            }
        }
        self.lines.push_back(line);
    }

    /// Queues "Object: error" for the channel while the rate allows it.
    fn queue_for_channel(&mut self, channel: i32, from: &str, text: &str, now: Instant) {
        let elapsed = self
            .refilled
            .map_or(CHANNEL_BURST, |t| now.duration_since(t).as_secs_f32() * CHANNEL_PER_SECOND);
        self.tokens = (self.tokens + elapsed).min(CHANNEL_BURST);
        self.refilled = Some(now);
        if self.tokens < 1.0 {
            log::debug!("script error not sent on channel {channel}: too many at once");
            return;
        }
        self.tokens -= 1.0;
        self.outgoing.push((channel, format!("{from}: {text}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn message(owner: Uuid, chat_type: ChatType) -> ChatMessage {
        ChatMessage {
            from_name: "Object".into(),
            source_id: Uuid::from_u128(2),
            owner_id: owner,
            source_type: ChatSourceType::Object,
            chat_type,
            region: None,
            position: glam::Vec3::ZERO,
            message: "hello".into(),
        }
    }

    /// A script window's Console says what the object it shows answered,
    /// whichever account owns it; the Debug window keeps the Legacy Viewer's
    /// rule (EXT-7762), our objects alone.
    #[test]
    fn the_console_keeps_the_lines_of_objects_that_are_not_ours() {
        let agent = Uuid::from_u128(1);
        let now = Instant::now();
        let mut messages = Messages::default();
        let mut said = message(agent, ChatType::OwnerSay);
        messages.route(&said, agent, now);
        said.owner_id = Uuid::from_u128(3);
        said.chat_type = ChatType::Debug;
        messages.route(&said, agent, now);
        assert_eq!(messages.lines.len(), 1, "the Debug window: our own line alone");
        assert_eq!(messages.console.len(), 2, "the Console: both");
    }

    #[test]
    fn debug_accepts_only_owned_objects_and_owner_say_routing_is_optional_and_bounded() {
        let agent = Uuid::from_u128(1);
        let now = Instant::now();
        let mut message = message(agent, ChatType::OwnerSay);
        let mut messages = Messages::default();
        // not routed to the window, but kept for the console
        assert!(!messages.route(&message, agent, now));
        assert_eq!(messages.lines.len(), 1);
        messages.owner_say = true;
        assert!(messages.route(&message, agent, now));
        message.owner_id = Uuid::from_u128(3);
        assert!(messages.route(&message, agent, now));
        assert_eq!(messages.lines.len(), 2, "another resident's line is not kept");
        message.owner_id = agent;
        messages.owner_say = false;
        message.chat_type = ChatType::Debug;
        for _ in 0..510 {
            assert!(messages.route(&message, agent, now));
        }
        assert_eq!(messages.lines.len(), 500);
        message.source_type = ChatSourceType::Agent;
        assert!(!messages.route(&message, agent, now));
    }

    /// A script that says a lot to its owner does not push the errors the
    /// window is for out of its 500 lines.
    #[test]
    fn the_oldest_line_of_a_kind_makes_room_for_a_new_one() {
        let agent = Uuid::from_u128(1);
        let now = Instant::now();
        let mut messages = Messages {
            owner_say: true,
            ..Default::default()
        };
        let error = message(agent, ChatType::Debug);
        messages.route(&error, agent, now);
        let mut said = message(agent, ChatType::OwnerSay);
        for _ in 0..600 {
            messages.route(&said, agent, now);
        }
        assert_eq!(messages.lines.len(), 500);
        assert!(
            messages
                .lines
                .iter()
                .any(|line| matches!(line.kind, crate::world::ChatKind::Object(ChatType::Debug))),
            "the error is still there"
        );
        said.message = "last".into();
        messages.route(&said, agent, now);
        assert_eq!(messages.lines.back().map(|line| line.text.as_str()), Some("last"));
    }

    /// In the nearby chat, our own script errors go on to the chat and stay
    /// in the window, which they no longer open; another resident's errors
    /// still show nowhere, and an owner-say sent to the window still opens it.
    #[test]
    fn errors_in_chat_go_on_to_the_chat_and_keep_the_window_closed() {
        let agent = Uuid::from_u128(1);
        let now = Instant::now();
        let mut messages = Messages {
            in_chat: true,
            owner_say: true,
            ..Default::default()
        };
        assert!(
            !messages.route(&message(agent, ChatType::Debug), agent, now),
            "on to the nearby chat"
        );
        assert_eq!((messages.lines.len(), messages.opens), (1, 0));
        assert!(
            messages.route(&message(Uuid::from_u128(3), ChatType::Debug), agent, now),
            "not ours: dropped"
        );
        assert_eq!(messages.lines.len(), 1);
        assert!(messages.route(&message(agent, ChatType::OwnerSay), agent, now));
        assert_eq!((messages.lines.len(), messages.opens), (2, 1));
        messages.in_chat = false;
        assert!(messages.route(&message(agent, ChatType::Debug), agent, now));
        assert_eq!((messages.lines.len(), messages.opens), (3, 2));
    }

    /// Only our own script errors are said on the channel, with their
    /// object's name, five at once then one a second.
    #[test]
    fn errors_go_on_the_channel_at_a_bounded_rate() {
        let agent = Uuid::from_u128(1);
        let start = Instant::now();
        let mut messages = Messages {
            in_chat: true,
            owner_say: true,
            ..Default::default()
        };
        messages.route(&message(agent, ChatType::Debug), agent, start);
        assert!(messages.outgoing.is_empty(), "no channel chosen");
        messages.channel = Some(-7);
        messages.route(&message(agent, ChatType::OwnerSay), agent, start);
        messages.route(&message(Uuid::from_u128(3), ChatType::Debug), agent, start);
        assert!(messages.outgoing.is_empty(), "owner-say and others' errors stay");
        for _ in 0..8 {
            messages.route(&message(agent, ChatType::Debug), agent, start);
        }
        assert_eq!(messages.outgoing.len(), 5);
        assert_eq!(messages.outgoing[0], (-7, "Object: hello".to_owned()));
        messages.route(&message(agent, ChatType::Debug), agent, start + Duration::from_millis(500));
        assert_eq!(messages.outgoing.len(), 5);
        messages.route(&message(agent, ChatType::Debug), agent, start + Duration::from_millis(1100));
        assert_eq!(messages.outgoing.len(), 6);
    }
}
