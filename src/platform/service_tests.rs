use super::*;
use std::sync::{Arc, Weak, mpsc::{self, Receiver}};
use std::time::Duration;

fn subscribe(owner: &str, name: &str, tag: &str) -> (Receiver<SysValue>, Weak<()>) {
    let (sender, receiver) = mpsc::channel();
    let lifetime = Arc::new(());
    let weak = Arc::downgrade(&lifetime);
    assert!(service(owner, name, tag, Box::new(move |value| {
        let _keep_alive = &lifetime;
        let _ = sender.send(value);
    })));
    (receiver, weak)
}

#[test]
fn a_recreated_owner_replaces_its_listener_and_gets_the_running_snapshot() {
    let owner = "test-recreated-clock-owner";
    let (first, mut previous) = subscribe(owner, "clock.seconds", "watch");
    assert!(matches!(first.recv_timeout(Duration::from_secs(3)).unwrap(), SysValue::Map(_)));
    for _ in 0..12 {
        let (replacement, alive) = subscribe(owner, "clock.seconds", "watch");
        assert!(previous.upgrade().is_none(), "the old callback is still retained");
        // A quiet service still initializes the new logic immediately.
        assert!(matches!(replacement.try_recv().unwrap(), SysValue::Map(_)));
        previous = alive;
    }
    assert_eq!(HUBS.lock().unwrap().iter().filter(|(key, _)| key.0 == owner).count(), 1);
}

#[test]
fn declarative_and_luau_subscriptions_do_not_replace_each_other() {
    let owner = "test-independent-clock-subscriptions";
    let (watch, watched) = subscribe(owner, "clock.seconds", "watch");
    let (declared, declaration) = subscribe(owner, "clock.seconds", "service:now");
    watch.recv_timeout(Duration::from_secs(3)).unwrap();
    declared.recv_timeout(Duration::from_secs(3)).unwrap();
    let (replacement, _) = subscribe(owner, "clock.seconds", "watch");
    replacement.try_recv().unwrap();
    assert!(watched.upgrade().is_none());
    assert!(declaration.upgrade().is_some());
    declared.recv_timeout(Duration::from_secs(3)).unwrap();
    replacement.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(HUBS.lock().unwrap().iter().filter(|(key, _)| key.0 == owner).count(), 2);
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
#[test]
fn application_catalog_replays_without_restarting_or_retaining_old_listeners() {
    let owner = "test-recreated-apps-owner";
    let (first, mut previous) = subscribe(owner, "apps", "watch");
    let initial = first.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(matches!(initial, SysValue::List(_)));
    for _ in 0..12 {
        let (replacement, alive) = subscribe(owner, "apps", "watch");
        assert!(previous.upgrade().is_none(), "apps retained the previous listener");
        let replay = replacement.try_recv().unwrap();
        assert!(matches!(replay, SysValue::List(_)));
        #[cfg(target_os = "linux")]
        assert_eq!(replay, initial);
        previous = alive;
    }
    assert_eq!(HUBS.lock().unwrap().iter().filter(|(key, _)| key.0 == owner).count(), 1);
}
