//! The screen stays on while a game shows (`Shell`). A controller's buttons aren't input to the
//! desktop's idle timer, so on Linux the screen dimmed and the computer suspended mid-game unless
//! the mouse moved (the user's v0.7.1 play test). The desktop portal's Inhibit asks the desktop
//! (GNOME, KDE and the others with a portal; the file chooser's portal) until it's dropped.

/// Keeps the screen on until dropped (elsewhere than Linux: nothing).
pub struct KeepAwake {
    #[cfg(all(unix, not(target_os = "macos")))]
    _stop: tokio::sync::oneshot::Sender<()>,
}

impl KeepAwake {
    pub fn start() -> Self {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            // Its own thread and runtime: the inhibition lasts until `stop` drops, then closes.
            let _ = std::thread::Builder::new()
                .name("keep-awake".into())
                .spawn(move || {
                    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    rt.block_on(async move {
                        use ashpd::desktop::inhibit::{InhibitFlags, InhibitProxy};
                        let flags = InhibitFlags::Idle | InhibitFlags::Suspend;
                        let asked = async {
                            InhibitProxy::new()
                                .await?
                                .inhibit(None, flags, "Playing GoldLink")
                                .await
                        };
                        // A portal without a backend can leave the call unanswered.
                        let wait = std::time::Duration::from_secs(5);
                        match tokio::time::timeout(wait, asked).await {
                            Ok(Ok(request)) => {
                                log::info!("the screen stays on while playing");
                                let _ = stopped.await;
                                let _ = request.close().await;
                            }
                            Ok(Err(e)) => log::warn!("the screen may dim while playing: {e}"),
                            Err(_) => log::warn!("the screen may dim while playing: no portal"),
                        }
                    });
                });
            Self { _stop: stop }
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        Self {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "asks the real desktop"]
    fn holds() {
        let k = super::KeepAwake::start();
        std::thread::sleep(std::time::Duration::from_secs(15));
        drop(k);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
