use std::sync::{atomic::AtomicU32, atomic::Ordering, Arc};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::{events::AppEvent, layout::PaneId};

// Lifecycle hooks can pause agent process probes, but foreground labels must still refresh.
pub(super) struct ForegroundProcessTask(tokio::task::AbortHandle);

impl Drop for ForegroundProcessTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl ForegroundProcessTask {
    pub(super) fn spawn(
        pane_id: PaneId,
        child_pid: Arc<AtomicU32>,
        events: mpsc::Sender<AppEvent>,
    ) -> Self {
        Self(
            tokio::spawn(async move {
                // Publish an unknown first observation too, to clear metadata from a replaced runtime.
                let mut previous = None;
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let pid = child_pid.load(Ordering::Acquire);
                    let observed = (pid > 0)
                        .then(|| crate::platform::foreground_process_observation(pid))
                        .flatten()
                        .map(|observation| {
                            (
                                sanitize_process_name(&observation.name),
                                observation.child.as_deref().and_then(sanitize_process_name),
                            )
                        })
                        .unwrap_or((None, None));
                    if previous.as_ref() != Some(&observed) {
                        if events
                            .send(AppEvent::ForegroundProcessChanged {
                                pane_id,
                                name: observed.0.clone(),
                                child_name: observed.1.clone(),
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        previous = Some(observed);
                    }
                }
            })
            .abort_handle(),
        )
    }
}

fn sanitize_process_name(value: &str) -> Option<String> {
    let name: String = value
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(character, '\u{2028}' | '\u{2029}')
                && !matches!(character, '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
        })
        .take(64)
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn monitor_clears_unknown_initial_process_and_stops_on_drop() {
        let (tx, mut rx) = mpsc::channel(1);
        let task = ForegroundProcessTask::spawn(PaneId::alloc(), Arc::new(AtomicU32::new(0)), tx);
        assert!(matches!(
            rx.recv().await,
            Some(AppEvent::ForegroundProcessChanged { name: None, .. })
        ));
        drop(task);
        assert!(tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn process_labels_are_bounded_and_cannot_contain_terminal_or_bidi_controls() {
        assert_eq!(
            sanitize_process_name(" \u{1b}\nvi\u{202e}m\u{200b}\u{7} "),
            Some("vim".into())
        );
        assert_eq!(sanitize_process_name("\n\u{202e} "), None);
        assert_eq!(
            sanitize_process_name(&"界".repeat(100))
                .unwrap()
                .chars()
                .count(),
            64
        );
        assert_eq!(
            sanitize_process_name("python3.14"),
            Some("python3.14".into())
        );
    }
}
