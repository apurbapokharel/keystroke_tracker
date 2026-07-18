use std::collections::HashMap;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use zbus::{Connection, proxy, zvariant::Value};

#[proxy(
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    interface = "org.freedesktop.login1.Manager"
)]
pub trait Login1Manager {
    // Defines signature for D-Bus signal named `PrepareForSleep`
    #[zbus(signal)]
    fn prepare_for_sleep(&self, status: bool);
}

#[proxy(
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait Notifications {
    /// Call the org.freedesktop.Notifications.Notify D-Bus method
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, &Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

/// Spawn a background task that turns error strings into desktop notifications,
/// and return a sender for feeding it messages.
///
/// This lets the synchronous input threads (which run under `spawn_blocking` and
/// therefore cannot `.await`) surface failures to the user: they just
/// `send(msg)` and this task does the async D-Bus call.
///
/// Notifications live on the **session** bus (separate from login1's system
/// bus) and are best-effort: if the session bus or a notification daemon isn't
/// available, messages are logged to stderr (journald) and dropped rather than
/// blocking or crashing the daemon.
pub fn spawn_notifier() -> UnboundedSender<String> {
    let (tx, mut rx) = unbounded_channel::<String>();

    tokio::spawn(async move {
        // Keep `conn` bound for the whole task so the proxy that borrows it
        // stays valid. If either step fails we degrade to logging only.
        let conn = match Connection::session().await {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!(
                    "notifier: no session bus ({e}); desktop notifications disabled, \
                     failures will be logged only"
                );
                None
            }
        };
        let proxy = match &conn {
            Some(c) => match NotificationsProxy::new(c).await {
                Ok(p) => Some(p),
                Err(e) => {
                    eprintln!("notifier: could not build notifications proxy: {e}");
                    None
                }
            },
            None => None,
        };

        while let Some(msg) = rx.recv().await {
            match &proxy {
                Some(p) => {
                    // urgency = 2 (critical) so the popup persists; must outlive
                    // the borrow held in `hints` until the call completes.
                    let urgency = Value::U8(2);
                    let mut hints: HashMap<&str, &Value> = HashMap::new();
                    hints.insert("urgency", &urgency);
                    if let Err(e) = p
                        .notify(
                            "tracker",
                            0,             // replaces_id: 0 = new notification
                            "dialog-error",
                            "tracker daemon",
                            &msg,
                            &[],           // no actions
                            hints,
                            0,             // expire_timeout: 0 = never auto-dismiss
                        )
                        .await
                    {
                        eprintln!("notifier: send failed: {e}; original message: {msg}");
                    }
                }
                None => eprintln!("notifier: {msg}"),
            }
        }
    });

    tx
}
