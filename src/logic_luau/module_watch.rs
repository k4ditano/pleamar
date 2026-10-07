//! Required files belong to the VM that read them, including each plugin.
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

static WATCHES: Mutex<Vec<Weak<Modules>>> = Mutex::new(Vec::new());

pub(super) struct Modules {
    scene: String,
    paths: Mutex<BTreeSet<PathBuf>>,
}

impl Modules {
    pub fn new(scene: &str) -> Arc<Self> {
        let modules = Arc::new(Self { scene: scene.to_owned(), paths: Mutex::default() });
        let mut watches = WATCHES.lock().unwrap();
        watches.retain(|w| w.strong_count() != 0);
        watches.push(Arc::downgrade(&modules));
        modules
    }

    pub fn add(&self, path: PathBuf) {
        self.paths.lock().unwrap().insert(path);
    }

    // A failed candidate may need a missing/broken module repaired. Keep
    // watching that file while the previous VM continues to serve the scene.
    pub fn retain_failed(&self, candidate: &Self) {
        let paths = candidate.paths.lock().unwrap().clone();
        self.paths.lock().unwrap().extend(paths);
    }
}

pub(crate) fn required(scene: &str) -> Vec<PathBuf> {
    let mut watches = WATCHES.lock().unwrap();
    let mut paths = BTreeSet::new();
    watches.retain(|watch| {
        let Some(modules) = watch.upgrade() else { return false };
        if modules.scene == scene {
            paths.extend(modules.paths.lock().unwrap().iter().cloned());
        }
        true
    });
    paths.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_and_plugins_release_their_own_module_watches() {
        let scene = "module-watch-lifetimes.plm";
        let main = Modules::new(scene);
        let plugin = Modules::new(scene);
        let other = Modules::new("module-watch-other.plm");
        main.add("main/util.luau".into());
        main.add("main/util.luau".into());
        plugin.add("plugin/util.luau".into());
        other.add("other/util.luau".into());
        assert_eq!(required(scene), vec![PathBuf::from("main/util.luau"), "plugin/util.luau".into()]);
        drop(plugin);
        assert_eq!(required(scene), vec![PathBuf::from("main/util.luau")]);
        drop(main);
        assert!(required(scene).is_empty());
    }

    #[test]
    fn failed_candidates_remain_repairable_until_a_successful_reload() {
        let scene = "module-watch-recovery.plm";
        let retained = Modules::new(scene);
        retained.add("old.luau".into());
        let candidate = Modules::new(scene);
        candidate.add("missing.luau".into());
        retained.retain_failed(&candidate);
        drop(candidate);
        assert_eq!(required(scene), vec![PathBuf::from("missing.luau"), "old.luau".into()]);
        let next = Modules::new(scene);
        next.add("new.luau".into());
        drop(retained);
        assert_eq!(required(scene), vec![PathBuf::from("new.luau")]);
    }
}
