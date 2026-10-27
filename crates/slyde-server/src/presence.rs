//! Which studio windows are connected right now.
//!
//! Every page the app opens (editor, presenter, speaker view) keeps one WebSocket
//! to the server, so the number of open sockets is the number of open windows.
//! The windowless launcher uses it to stop the server when the last window is
//! closed; the console build and MCP-started servers ignore it and keep running.

use std::time::Duration;

use tokio::sync::watch;

/// Counts connected windows and lets a task wait for all of them to leave.
#[derive(Debug)]
pub struct Presence {
    count: watch::Sender<usize>,
}

impl Default for Presence {
    fn default() -> Self {
        Self {
            count: watch::channel(0).0,
        }
    }
}

/// Held for as long as one window is connected; dropping it leaves.
#[derive(Debug)]
pub struct PresenceGuard {
    count: watch::Sender<usize>,
}

impl Presence {
    /// Registers a connected window.
    pub fn join(&self) -> PresenceGuard {
        // `send_modify` updates the value even when nobody is subscribed yet,
        // which `send` would silently refuse to do.
        self.count.send_modify(|count| *count += 1);
        PresenceGuard {
            count: self.count.clone(),
        }
    }

    /// How many windows are connected.
    pub fn connected(&self) -> usize {
        *self.count.borrow()
    }

    /// A receiver that wakes on every join and leave.
    pub fn subscribe(&self) -> watch::Receiver<usize> {
        self.count.subscribe()
    }
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        self.count
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// Why [`wait_until_abandoned`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abandoned {
    /// No window ever connected, so the browser never opened the app.
    NeverOpened,
    /// A window connected and every window has been gone for the whole grace period.
    Closed,
}

/// Waits until the app has been opened and then closed again.
///
/// `first_window` bounds how long the very first connection may take. `grace`
/// is how long the count has to stay at zero before the app counts as closed;
/// a reload or a switch between the editor and the presenter drops the socket
/// for a moment and must not stop the server.
pub async fn wait_until_abandoned(
    mut updates: watch::Receiver<usize>,
    first_window: Duration,
    grace: Duration,
) -> Abandoned {
    let opened = tokio::time::timeout(first_window, updates.wait_for(|count| *count > 0))
        .await
        .is_ok_and(|reached| reached.is_ok());
    if !opened {
        return Abandoned::NeverOpened;
    }

    loop {
        // The sender lives as long as the server; if it is gone, so is the app.
        if updates.wait_for(|count| *count == 0).await.is_err() {
            return Abandoned::Closed;
        }
        // Zero for the whole grace period, or somebody came back.
        let came_back = tokio::time::timeout(grace, updates.wait_for(|count| *count > 0))
            .await
            .is_ok();
        if !came_back {
            return Abandoned::Closed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(60);
    const LONG: Duration = Duration::from_secs(5);

    #[test]
    fn joining_and_leaving_keeps_the_count_exact() {
        let presence = Presence::default();
        assert_eq!(presence.connected(), 0);
        let first = presence.join();
        let second = presence.join();
        assert_eq!(presence.connected(), 2);
        drop(first);
        assert_eq!(presence.connected(), 1);
        drop(second);
        assert_eq!(presence.connected(), 0);
    }

    #[tokio::test]
    async fn a_browser_that_never_connects_is_reported() {
        let presence = Presence::default();
        let outcome = wait_until_abandoned(presence.subscribe(), SHORT, SHORT).await;
        assert_eq!(outcome, Abandoned::NeverOpened);
    }

    #[tokio::test]
    async fn the_last_window_closing_ends_the_session_after_the_grace_period() {
        let presence = Presence::default();
        let guard = presence.join();
        let waiter = tokio::spawn(wait_until_abandoned(presence.subscribe(), LONG, SHORT));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "an open window keeps the server up");
        drop(guard);
        let outcome = tokio::time::timeout(LONG, waiter).await.unwrap().unwrap();
        assert_eq!(outcome, Abandoned::Closed);
    }

    #[tokio::test]
    async fn a_window_that_comes_back_inside_the_grace_period_keeps_the_server_up() {
        let presence = Presence::default();
        let guard = presence.join();
        let waiter = tokio::spawn(wait_until_abandoned(
            presence.subscribe(),
            LONG,
            Duration::from_millis(400),
        ));
        tokio::time::sleep(Duration::from_millis(20)).await;
        // A reload: the socket drops and a new one arrives right away.
        drop(guard);
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _again = presence.join();
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!waiter.is_finished(), "a reload must not stop the server");
    }

    #[tokio::test]
    async fn two_windows_need_both_to_close() {
        let presence = Presence::default();
        let editor = presence.join();
        let presenter = presence.join();
        let waiter = tokio::spawn(wait_until_abandoned(presence.subscribe(), LONG, SHORT));
        drop(editor);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!waiter.is_finished(), "the presenter is still open");
        drop(presenter);
        let outcome = tokio::time::timeout(LONG, waiter).await.unwrap().unwrap();
        assert_eq!(outcome, Abandoned::Closed);
    }
}
