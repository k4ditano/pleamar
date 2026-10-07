use super::*;

fn fixture() -> Scene {
    crate::scenes::from_file::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/still-monitor.plm")).unwrap()
}

fn sample(scene: &Scene, states: &mut [RuleState], copy: usize, value: f32, now: Instant) -> (bool, bool) {
    let mut result = (false, false);
    for k in scene.spans[copy].rules.clone() {
        let Some(state) = sampled_rule(scene, states, k, now) else { continue };
        match scene.rules[k].when {
            Trigger::Change(_) => { result.0 |= state.changed(value); }
            Trigger::Still { duration, .. } => { result.1 |= state.still(value, duration, now, &mut Vec::new()); }
            _ => {}
        }
    }
    result
}

#[test]
fn volume_timeout_survives_a_move_to_an_unseen_monitor() {
    let scene = fixture();
    let mut states: Vec<_> = scene.rules.iter().map(|_| RuleState::default()).collect();
    let start = Instant::now();
    assert_eq!(sample(&scene, &mut states, 0, 0.0, start), (false, false));
    assert_eq!(sample(&scene, &mut states, 0, 1.0, start + Duration::from_millis(100)), (true, false));
    // Marea's shared meter is now visible. Copy 0 closes before its deadline;
    // copy 1 has never sampled the volume, but must finish the same episode.
    assert_eq!(sample(&scene, &mut states, 1, 1.0, start + Duration::from_millis(400)), (false, false));
    assert!(sample(&scene, &mut states, 1, 1.0, start + Duration::from_millis(1201)).1,
        "the volume meter lost its hide timer when the active monitor changed");
    assert!(!sample(&scene, &mut states, 0, 1.0, start + Duration::from_millis(1400)).1,
        "returning to the old monitor must not replay an expired timer");
}

#[test]
fn both_visible_copies_sample_shared_changes_once_per_frame() {
    let scene = fixture();
    let shared: Vec<_> = scene.rules.iter().enumerate().filter_map(|(k, r)|
        matches!(r.when, Trigger::Still { .. }).then_some(k)).collect();
    assert_eq!(shared.len(), 2);
    assert_eq!(scene.twin_of[shared[1]], shared[0]);
    let mut states: Vec<_> = scene.rules.iter().map(|_| RuleState::default()).collect();
    let now = Instant::now();
    assert!(sampled_rule(&scene, &mut states, shared[0], now).is_some());
    assert!(sampled_rule(&scene, &mut states, shared[1], now).is_none());
    assert!(sampled_rule(&scene, &mut states, shared[1], now + Duration::from_millis(16)).is_some());
}

#[test]
fn per_monitor_expressions_keep_separate_histories() {
    let scene = fixture();
    let unique: Vec<_> = scene.rules.iter().enumerate().filter_map(|(k, r)|
        (matches!(r.when, Trigger::Change(_)) && r.effects.is_empty()).then_some(k)).collect();
    assert_eq!(unique.len(), 2);
    let mut states: Vec<_> = scene.rules.iter().map(|_| RuleState::default()).collect();
    let now = Instant::now();
    for (i, &k) in unique.iter().enumerate() {
        assert_eq!(scene.twin_of[k], k);
        assert!(!sampled_rule(&scene, &mut states, k, now).unwrap().changed(i as f32));
    }
}

#[test]
fn volume_burst_hides_once_after_last_change_not_initial_snapshot() {
    let mut state = RuleState::default();
    let mut appointments = Vec::new();
    let start = Instant::now();
    let wait = Duration::from_millis(1100);
    assert!(!state.still(1.0, wait, start, &mut appointments));
    assert!(appointments.is_empty());
    assert!(!state.still(0.8, wait, start + Duration::from_millis(100), &mut appointments));
    assert!(!state.still(0.6, wait, start + Duration::from_millis(700), &mut appointments));
    assert!(!state.still(0.6, wait, start + Duration::from_millis(1201), &mut appointments));
    assert!(state.still(0.6, wait, start + Duration::from_millis(1801), &mut appointments));
    assert!(!state.still(0.6, wait, start + Duration::from_millis(2000), &mut appointments));
}
