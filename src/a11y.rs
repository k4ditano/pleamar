//! Screen readers: each surface told as an accessibility tree (AccessKit; on
//! Linux, AT-SPI). It is the tree `describe` gives an agent (`agent.rs`), seen
//! as the person would: what is kept from agents (`agent: no`) is there, and
//! can be pressed —the person is the one asking—; a secret field's words never
//! are. See `docs/13-screen-readers.md`.
//!
//! Nothing is built nor sent while no reader is listening: each surface only
//! registers with AccessKit, whose own thread waits for one to ask. When one
//! does, the render looks at the scene at most ten times a second, and only
//! after something changed, and sends what changed in the tree.

/// What a surface's reader says or asks, as the render takes it (no AccessKit
/// types: the render matches it whether the feature is in or not).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A reader is listening to that sheet: it wants the whole tree.
    Listening(u32),
    /// It has gone: nothing more to build for that sheet.
    Gone(u32),
    /// Do that to that node (a zone of the scene: `zone_of`).
    Act(u32, u64, Ask),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// Pressed, as a person's hand presses it.
    Press,
    /// Brought to the reader's attention: a field starts being written in.
    Focus,
    /// A field's words, replaced (as if typed over them).
    Value(String),
    /// A slider one step up (1) or down (−1): a notch of the wheel over it.
    Step(f32),
}

/// The window itself: every surface's tree hangs from it.
#[cfg_attr(not(all(target_os = "linux", feature = "accessibility")), allow(dead_code))]
pub const ROOT: u64 = 1;
/// A zone's node: its place among the scene's zones, past the root.
pub fn node_of_zone(k: usize) -> u64 {
    k as u64 + 2
}
/// And back: the zone a node is, if it is one.
pub fn zone_of(node: u64) -> Option<usize> {
    (2..TEXTS).contains(&node).then(|| (node - 2) as usize)
}
/// Words drawn outside every zone: from here on, by where they were written.
const TEXTS: u64 = 1 << 40;

#[cfg(all(target_os = "linux", feature = "accessibility"))]
pub use reader::Readers;

#[cfg(not(all(target_os = "linux", feature = "accessibility")))]
pub use none::Readers;

/// Without the feature (or off Linux): the same calls, doing nothing.
#[cfg(not(all(target_os = "linux", feature = "accessibility")))]
mod none {
    #[derive(Default)]
    pub struct Readers;
    impl Readers {
        pub fn add(&mut self, _sheet: u32, _name: &str, _to_render: &std::sync::mpsc::Sender<crate::scene::ToRender>) {}
        pub fn remove(&mut self, _sheet: u32) {}
        pub fn listening(&self) -> bool {
            false
        }
        pub fn heard(&mut self, _e: &super::Event) {}
        pub fn focused(&mut self, _sheet: u32, _focused: bool) {}
        pub fn send(&mut self, _sheet: u32, _part: &crate::agent::Part, _focus: Option<u64>, _scale: f32) {}
    }
}

#[cfg(all(target_os = "linux", feature = "accessibility"))]
mod reader {
    use super::{Ask, Event, ROOT, TEXTS};
    use crate::agent::{Part, Role};
    use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Rect, Toggled, TreeId, TreeInfo, TreeUpdate};
    use std::collections::HashMap;
    use std::sync::mpsc::Sender;

    /// Each sheet's adapter, and what its reader was last sent.
    #[derive(Default)]
    pub struct Readers {
        sheets: HashMap<u32, Sheet>,
    }

    struct Sheet {
        adapter: accesskit_unix::Adapter,
        listening: bool,
        /// What the reader has, node by node: only what differs is sent again.
        sent: HashMap<u64, Node>,
        focus: u64,
        /// Where the reader last put its attention (`Focus` on something that is not a field).
        asked_focus: Option<u64>,
    }

    struct Listen(u32, Sender<crate::scene::ToRender>);
    impl accesskit::ActivationHandler for Listen {
        // Built on the render's thread, not here: the scene lives there. It is
        // sent before the next frame would be (`Readers::send`).
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            let _ = self.1.send(crate::scene::ToRender::Reader(Event::Listening(self.0)));
            None
        }
    }
    struct Gone(u32, Sender<crate::scene::ToRender>);
    impl accesskit::DeactivationHandler for Gone {
        fn deactivate_accessibility(&mut self) {
            let _ = self.1.send(crate::scene::ToRender::Reader(Event::Gone(self.0)));
        }
    }
    struct Acts(u32, Sender<crate::scene::ToRender>);
    impl accesskit::ActionHandler for Acts {
        fn do_action(&mut self, r: ActionRequest) {
            let node = r.target_node.0;
            let ask = match (r.action, r.data) {
                (Action::Click, _) => Ask::Press,
                (Action::Focus, _) => Ask::Focus,
                (Action::SetValue, Some(ActionData::Value(v))) => Ask::Value(v.into()),
                (Action::SetValue, Some(ActionData::NumericValue(_))) => return,
                (Action::Increment, _) => Ask::Step(1.0),
                (Action::Decrement, _) => Ask::Step(-1.0),
                _ => return,
            };
            let _ = self.1.send(crate::scene::ToRender::Reader(Event::Act(self.0, node, ask)));
        }
    }

    impl Readers {
        /// A sheet that a reader may come to: registered, and nothing else until one does.
        pub fn add(&mut self, sheet: u32, _name: &str, to_render: &Sender<crate::scene::ToRender>) {
            let adapter = accesskit_unix::Adapter::new(Listen(sheet, to_render.clone()), Acts(sheet, to_render.clone()), Gone(sheet, to_render.clone()));
            self.sheets.insert(sheet, Sheet { adapter, listening: false, sent: HashMap::new(), focus: ROOT, asked_focus: None });
        }

        pub fn remove(&mut self, sheet: u32) {
            self.sheets.remove(&sheet);
        }

        /// Whether any reader is listening: the only thing looked at every round.
        pub fn listening(&self) -> bool {
            self.sheets.values().any(|s| s.listening)
        }

        pub fn heard(&mut self, e: &Event) {
            match e {
                // Asked for the whole tree: what it had is forgotten, so all of it goes.
                Event::Listening(id) => {
                    if let Some(s) = self.sheets.get_mut(id) {
                        s.listening = true;
                        s.sent.clear();
                    }
                }
                Event::Gone(id) => {
                    if let Some(s) = self.sheets.get_mut(id) {
                        s.listening = false;
                        s.sent.clear();
                    }
                }
                Event::Act(id, node, Ask::Focus) => {
                    if let Some(s) = self.sheets.get_mut(id) {
                        s.asked_focus = Some(*node);
                    }
                }
                Event::Act(..) => {}
            }
        }

        /// The compositor gave the surface the keyboard, or took it.
        pub fn focused(&mut self, sheet: u32, focused: bool) {
            if let Some(s) = self.sheets.get_mut(&sheet) {
                s.adapter.update_window_focus_state(focused);
            }
        }

        /// A sheet's tree as it is now, to its reader: all of it the first time,
        /// then what changed. `focus`: the field being written in, if it is on it.
        pub fn send(&mut self, sheet: u32, part: &Part, focus: Option<u64>, scale: f32) {
            let Some(s) = self.sheets.get_mut(&sheet).filter(|s| s.listening) else { return };
            let nodes = tree(part, scale);
            let focus = focus.or(s.asked_focus).filter(|f| nodes.iter().any(|(id, _)| id == f)).unwrap_or(ROOT);
            let whole = s.sent.is_empty();
            let changed = changes(&s.sent, &nodes);
            if changed.is_empty() && focus == s.focus {
                return;
            }
            let update = TreeUpdate {
                nodes: changed.iter().map(|(id, n)| (NodeId(*id), n.clone())).collect(),
                tree: whole.then(|| {
                    let mut t = TreeInfo::new(NodeId(ROOT));
                    t.toolkit_name = Some("pleamar".into());
                    t.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
                    t
                }),
                tree_id: TreeId::ROOT,
                focus: NodeId(focus),
            };
            s.adapter.update_if_active(|| update);
            s.sent = nodes.into_iter().collect();
            s.focus = focus;
        }
    }

    /// What the reader does not have yet: the nodes that are new or differ from what it was sent.
    /// (A node gone is gone from its parent's children, and the parent differs.)
    pub(crate) fn changes(sent: &HashMap<u64, Node>, nodes: &[(u64, Node)]) -> Vec<(u64, Node)> {
        nodes.iter().filter(|(id, n)| sent.get(id) != Some(n)).cloned().collect()
    }

    /// The nodes of a surface's tree: the window, and in it what `describe` sees.
    pub(crate) fn tree(part: &Part, scale: f32) -> Vec<(u64, Node)> {
        let id_of = ids(part);
        let mut children: Vec<Vec<NodeId>> = vec![Vec::new(); part.nodes.len()];
        let mut top: Vec<NodeId> = Vec::new();
        for (i, n) in part.nodes.iter().enumerate() {
            match n.inside {
                Some(j) => children[j].push(NodeId(id_of[i])),
                None => top.push(NodeId(id_of[i])),
            }
        }
        let mut out = Vec::with_capacity(part.nodes.len() + 1);
        let mut window = Node::new(accesskit::Role::Window);
        window.set_label(part.name.as_str());
        window.set_bounds(Rect { x0: 0.0, y0: 0.0, x1: (part.size.0 * scale) as f64, y1: (part.size.1 * scale) as f64 });
        window.set_children(top);
        out.push((ROOT, window));
        for (i, (n, kids)) in part.nodes.iter().zip(children).enumerate() {
            out.push((id_of[i], node(n, kids, scale)));
        }
        out
    }

    /// Each node's id, stable from look to look: a zone by its place among the
    /// scene's zones; loose words by where they were written (and, written twice by
    /// the same instruction, in order).
    fn ids(part: &Part) -> Vec<u64> {
        let mut seen: HashMap<usize, u64> = HashMap::new();
        part.nodes
            .iter()
            .map(|n| {
                if n.zone != usize::MAX {
                    return super::node_of_zone(n.zone);
                }
                let k = seen.entry(n.order).or_insert(0);
                *k += 1;
                TEXTS + ((n.order as u64) << 8) + (*k - 1).min(255)
            })
            .collect()
    }

    fn node(n: &crate::agent::Node, children: Vec<NodeId>, scale: f32) -> Node {
        let secret = n.secret;
        let role = match n.role {
            Role::Button => accesskit::Role::Button,
            // A toggle that says whether it is on: a switch; one that does not, a button that toggles.
            Role::Toggle if n.checked.is_some() => accesskit::Role::Switch,
            Role::Toggle => accesskit::Role::Button,
            Role::Tab => accesskit::Role::Tab,
            Role::Link => accesskit::Role::Link,
            Role::Item => accesskit::Role::ListItem,
            Role::Slider => accesskit::Role::Slider,
            Role::Field if secret => accesskit::Role::PasswordInput,
            Role::Field => accesskit::Role::TextInput,
            Role::List => accesskit::Role::List,
            Role::Region => accesskit::Role::Group,
            Role::Window => accesskit::Role::Window,
            Role::Text => accesskit::Role::Label,
        };
        let mut node = Node::new(role);
        // Words drawn on their own are a label's text: AccessKit reads that from its value.
        if n.role == Role::Text {
            node.set_value(n.label.as_str());
        } else if !n.label.is_empty() {
            node.set_label(n.label.as_str());
        }
        match (&n.value, n.role) {
            (Some(_), Role::Field) if secret => {}
            (Some(v), Role::Slider) => match v.parse::<f64>() {
                Ok(x) => node.set_numeric_value(x),
                Err(_) => node.set_value(v.as_str()),
            },
            (Some(v), _) => node.set_value(v.as_str()),
            (None, _) => {}
        }
        if let Some(c) = n.checked {
            node.set_toggled(if c { Toggled::True } else { Toggled::False });
        }
        if let Some(s) = n.selected {
            node.set_selected(s);
        }
        if n.inactive {
            node.set_disabled();
        }
        let [x, y, w, h] = n.at.map(|v| (v * scale) as f64);
        node.set_bounds(Rect { x0: x, y0: y, x1: x + w, y1: y + h });
        if !children.is_empty() {
            node.set_children(children);
        }
        // What can be done to it, as a person's hand does it.
        if !n.inactive {
            match n.role {
                Role::Button | Role::Toggle | Role::Tab | Role::Link | Role::Item => node.add_action(Action::Click),
                Role::Field => {
                    node.add_action(Action::Focus);
                    node.add_action(Action::Click);
                    if !secret {
                        node.add_action(Action::SetValue);
                    }
                }
                Role::Slider => {
                    node.add_action(Action::Increment);
                    node.add_action(Action::Decrement);
                }
                _ => {}
            }
        }
        node
    }

}

#[cfg(all(test, target_os = "linux", feature = "accessibility"))]
mod tests {
    use super::reader::{changes, tree};
    use crate::agent::{describe, Shown, Sight};
    use crate::scene::{Animated, Ctx};
    use accesskit::{Action, Role, Toggled};

    /// `tests/agent.plm` as a person's reader sees it, dirty or not.
    fn parts(dirty: bool) -> Vec<crate::agent::Part> {
        let (scene, _) = crate::language::read_file(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/agent.plm")).unwrap();
        let props: Vec<Animated> = scene.props.iter().map(|(_, v, s)| Animated { x: *v, v: 0.0, target: *v, spring: *s }).collect();
        let mut facts: Vec<f32> = scene.facts.iter().map(|f| f.1).collect();
        if dirty {
            facts[scene.facts.iter().position(|f| f.0 == "dirty").unwrap()] = 1.0;
        }
        let texts: Vec<String> = scene.texts.iter().map(|t| t.1.clone()).collect();
        let shown = scene.surfaces.iter().enumerate().map(|(k, s)| Shown { surface: k, popup: None, bounds: [s.origin.0, s.origin.1, s.origin.0 + s.width as f32, s.origin.1 + s.height as f32], scale: 1.0 }).collect();
        let never = |_: usize| false;
        let sight = Sight { shown, texts_seen: &[], gone: &never, rank: None, zones: None, index: None, person: true };
        describe(&scene, Ctx { props: &props, facts: &facts, seen: None }, &texts, &sight)
    }

    #[test]
    fn a_reader_is_told_each_thing_by_its_role_and_name() {
        let parts = parts(false);
        let main = &parts[0];
        let nodes = tree(main, 1.0);
        let said: Vec<(Role, String)> = nodes.iter().map(|(_, n)| (n.role(), n.label().unwrap_or("").to_owned())).collect();
        let has = |role: Role, name: &str| said.iter().any(|(r, l)| *r == role && l == name);
        assert_eq!(said[0], (Role::Window, "main".to_owned()), "{said:?}");
        assert!(has(Role::TextInput, "Search notes"), "{said:?}");
        assert!(has(Role::Slider, "Brightness") && has(Role::Slider, "Volume") && has(Role::Slider, "Microphone"), "{said:?}");
        assert!(has(Role::Switch, "Wi-Fi"), "{said:?}");
        assert!(has(Role::Tab, "Notes"), "{said:?}");
        assert!(has(Role::ListItem, "A note"), "{said:?}");
        assert!(has(Role::Group, "Drop here"), "{said:?}");
        // Kept from agents (`agent: no`), not from the person: there, and it can be pressed.
        let delete = nodes.iter().find(|(_, n)| n.label() == Some("Delete")).expect("Delete is there for the person");
        assert!(delete.1.supports_action(Action::Click));
        // A secret field never says its words.
        let pin = nodes.iter().find(|(_, n)| n.role() == Role::PasswordInput).expect("the pin, as a password");
        assert!(pin.1.value().is_none() && !pin.1.supports_action(Action::SetValue));
        // What it says it is.
        let wifi = nodes.iter().find(|(_, n)| n.label() == Some("Wi-Fi")).unwrap();
        assert_eq!(wifi.1.toggled(), Some(Toggled::True));
        assert_eq!(wifi.1.value(), Some("loud"));
        let notes = nodes.iter().find(|(_, n)| n.label() == Some("Notes")).unwrap();
        assert_eq!(notes.1.is_selected(), Some(true));
        let volume = nodes.iter().find(|(_, n)| n.label() == Some("Volume")).unwrap();
        assert!(volume.1.supports_action(Action::Increment) && volume.1.supports_action(Action::Decrement));
        // Each node once, and every one but the window hangs from something.
        let ids: std::collections::HashSet<u64> = nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), nodes.len());
        let hung: usize = nodes.iter().map(|(_, n)| n.children().len()).sum();
        assert_eq!(hung, nodes.len() - 1);
    }

    #[test]
    fn what_changed_is_all_a_reader_is_sent_again() {
        let before = tree(&parts(false)[0], 1.0);
        let after = tree(&parts(true)[0], 1.0);
        let sent: std::collections::HashMap<u64, accesskit::Node> = before.into_iter().collect();
        let changed = changes(&sent, &after);
        // Save turns active: it is new, and the window that holds it changed; nothing else.
        let names: Vec<String> = changed.iter().map(|(_, n)| n.label().unwrap_or("").to_owned()).collect();
        assert!(names.iter().any(|n| n.starts_with("Save")), "{names:?}");
        assert!(changed.iter().any(|(id, _)| *id == super::ROOT), "{names:?}");
        assert!(changed.len() <= 3, "{names:?}");
        // The same scene twice: nothing to send.
        assert!(changes(&after.iter().cloned().collect(), &after).is_empty());
    }
}
