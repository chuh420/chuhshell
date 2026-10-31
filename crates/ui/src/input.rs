use std::collections::HashMap;

use platform::{CursorShape, MouseButton};

use crate::tree::{NodeId, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AreaId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl Buttons {
    pub const NONE: Buttons = Buttons {
        left: false,
        right: false,
        middle: false,
    };
    pub const LEFT: Buttons = Buttons {
        left: true,
        right: false,
        middle: false,
    };
    pub const LEFT_RIGHT: Buttons = Buttons {
        left: true,
        right: true,
        middle: false,
    };

    fn accepts(self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.left,
            MouseButton::Right => self.right,
            MouseButton::Middle => self.middle,
            MouseButton::Other(_) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseArea {
    pub node: NodeId,
    pub hover: bool,
    pub buttons: Buttons,
    pub wheel: bool,
    pub cursor: CursorShape,
    pub enabled: bool,
}

impl MouseArea {
    pub fn new(node: NodeId) -> Self {
        Self {
            node,
            hover: true,
            buttons: Buttons::LEFT,
            wheel: false,
            cursor: CursorShape::Pointer,
            enabled: true,
        }
    }

    pub fn hover_handler(node: NodeId) -> Self {
        Self {
            node,
            hover: true,
            buttons: Buttons::NONE,
            wheel: false,
            cursor: CursorShape::Default,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Enter(AreaId),
    Leave(AreaId),
    Press(AreaId, MouseButton),
    Release(AreaId, MouseButton),
    Click(AreaId, MouseButton),
    Wheel(AreaId, i32),
}

struct Grab {
    area: AreaId,
    button: MouseButton,
}

#[derive(Default)]
pub struct Input {
    areas: HashMap<AreaId, MouseArea>,
    next: u32,
    position: Option<(f32, f32)>,
    hovered: Vec<AreaId>,
    grab: Option<Grab>,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, area: MouseArea) -> AreaId {
        let id = AreaId(self.next);
        self.next += 1;
        self.areas.insert(id, area);
        id
    }

    pub fn remove(&mut self, id: AreaId) {
        self.areas.remove(&id);
        self.hovered.retain(|area| *area != id);
        if self.grab.as_ref().is_some_and(|grab| grab.area == id) {
            self.grab = None;
        }
    }

    pub fn node(&self, id: AreaId) -> Option<NodeId> {
        self.areas.get(&id).map(|area| area.node)
    }

    pub fn get_mut(&mut self, id: AreaId) -> Option<&mut MouseArea> {
        self.areas.get_mut(&id)
    }

    pub fn contains_mouse(&self, id: AreaId) -> bool {
        self.hovered.contains(&id)
    }

    pub fn pressed(&self, id: AreaId) -> bool {
        self.grab.as_ref().is_some_and(|grab| grab.area == id)
    }

    pub fn cursor(&self) -> CursorShape {
        if let Some(grab) = &self.grab {
            return self
                .areas
                .get(&grab.area)
                .map_or(CursorShape::Default, |area| area.cursor);
        }
        self.hovered
            .iter()
            .filter_map(|id| self.areas.get(id))
            .map(|area| area.cursor)
            .find(|cursor| *cursor != CursorShape::Default)
            .unwrap_or_default()
    }

    fn stack(&self, tree: &Tree, x: f32, y: f32) -> Vec<AreaId> {
        let order: HashMap<NodeId, usize> = tree
            .paint_order()
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        let mut hits: Vec<(usize, AreaId)> = self
            .areas
            .iter()
            .filter(|(_, area)| area.enabled)
            .filter_map(|(id, area)| order.get(&area.node).map(|index| (*index, *id, area.node)))
            .filter(|(_, _, node)| tree.hit(*node, x, y))
            .map(|(index, id, _)| (index, id))
            .collect();
        hits.sort_by(|a, b| b.cmp(a));
        hits.into_iter().map(|(_, id)| id).collect()
    }

    fn hover_set(&self, tree: &Tree, x: f32, y: f32) -> Vec<AreaId> {
        let stack = self.stack(tree, x, y);
        let Some(top) = stack
            .iter()
            .copied()
            .find(|id| self.areas.get(id).is_some_and(|area| area.hover))
        else {
            return Vec::new();
        };
        let Some(top_node) = self.areas.get(&top).map(|area| area.node) else {
            return Vec::new();
        };
        let mut set: Vec<AreaId> = stack
            .into_iter()
            .filter(|id| {
                self.areas
                    .get(id)
                    .is_some_and(|area| area.hover && tree.is_ancestor_or_self(area.node, top_node))
            })
            .collect();
        set.sort();
        set
    }

    fn update_hover(&mut self, tree: &Tree, events: &mut Vec<InputEvent>) {
        let next = match self.position {
            Some((x, y)) => self.hover_set(tree, x, y),
            None => Vec::new(),
        };
        for id in &self.hovered {
            if !next.contains(id) {
                events.push(InputEvent::Leave(*id));
            }
        }
        for id in &next {
            if !self.hovered.contains(id) {
                events.push(InputEvent::Enter(*id));
            }
        }
        self.hovered = next;
    }

    pub fn motion(&mut self, tree: &Tree, x: f32, y: f32) -> Vec<InputEvent> {
        self.position = Some((x, y));
        let mut events = Vec::new();
        self.update_hover(tree, &mut events);
        events
    }

    pub fn refresh(&mut self, tree: &Tree) -> Vec<InputEvent> {
        let mut events = Vec::new();
        self.update_hover(tree, &mut events);
        events
    }

    pub fn leave(&mut self, tree: &Tree) -> Vec<InputEvent> {
        self.position = None;
        let mut events = Vec::new();
        self.update_hover(tree, &mut events);
        events
    }

    pub fn press(&mut self, tree: &Tree, button: MouseButton, x: f32, y: f32) -> Vec<InputEvent> {
        self.position = Some((x, y));
        let mut events = Vec::new();
        self.update_hover(tree, &mut events);
        if self.grab.is_some() {
            return events;
        }
        let target = self.stack(tree, x, y).into_iter().find(|id| {
            self.areas
                .get(id)
                .is_some_and(|area| area.buttons.accepts(button))
        });
        if let Some(area) = target {
            self.grab = Some(Grab { area, button });
            events.push(InputEvent::Press(area, button));
        }
        events
    }

    pub fn release(&mut self, tree: &Tree, button: MouseButton, x: f32, y: f32) -> Vec<InputEvent> {
        self.position = Some((x, y));
        let mut events = Vec::new();
        if let Some(grab) = self.grab.take_if(|grab| grab.button == button) {
            events.push(InputEvent::Release(grab.area, button));
            let inside = self
                .areas
                .get(&grab.area)
                .is_some_and(|area| tree.hit(area.node, x, y));
            if inside {
                events.push(InputEvent::Click(grab.area, button));
            }
        }
        self.update_hover(tree, &mut events);
        events
    }

    pub fn wheel(&mut self, tree: &Tree, x: f32, y: f32, delta: i32) -> Vec<InputEvent> {
        self.position = Some((x, y));
        let target = self
            .stack(tree, x, y)
            .into_iter()
            .find(|id| self.areas.get(id).is_some_and(|area| area.wheel));
        target
            .map(|area| vec![InputEvent::Wheel(area, delta)])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Node;

    fn setup() -> (Tree, Input, AreaId, NodeId) {
        let mut tree = Tree::new();
        let root = tree.root();
        let mut body = Node::new(0.0, 0.0, 1000.0, 100.0);
        body.scale = 1.22;
        body.origin = Some((0.0, 0.0));
        let body = tree.add(root, body);
        let chip = tree.add(body, Node::new(10.0, 3.0, 23.0, 22.0));
        let mut input = Input::new();
        let mut area = MouseArea::new(chip);
        area.buttons = Buttons::LEFT_RIGHT;
        let id = input.add(area);
        (tree, input, id, chip)
    }

    #[test]
    fn hit_respects_scale() {
        let (tree, mut input, id, _) = setup();
        let events = input.motion(&tree, 12.2 + 1.0, 3.66 + 1.0);
        assert_eq!(events, vec![InputEvent::Enter(id)]);
        let events = input.motion(&tree, (10.0 + 23.0) * 1.22 + 1.0, 10.0);
        assert_eq!(events, vec![InputEvent::Leave(id)]);
    }

    #[test]
    fn click_only_when_released_inside() {
        let (tree, mut input, id, _) = setup();
        let events = input.press(&tree, MouseButton::Left, 20.0, 10.0);
        assert!(events.contains(&InputEvent::Press(id, MouseButton::Left)));
        assert!(input.pressed(id));
        let events = input.release(&tree, MouseButton::Left, 20.0, 10.0);
        assert!(events.contains(&InputEvent::Click(id, MouseButton::Left)));
        input.press(&tree, MouseButton::Right, 20.0, 10.0);
        input.motion(&tree, 500.0, 50.0);
        assert!(input.pressed(id));
        let events = input.release(&tree, MouseButton::Right, 500.0, 50.0);
        assert!(events.contains(&InputEvent::Release(id, MouseButton::Right)));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, InputEvent::Click(..)))
        );
    }

    #[test]
    fn handler_and_area_on_same_node_both_hover() {
        let (tree, mut input, id, chip) = setup();
        let handler = input.add(MouseArea::hover_handler(chip));
        input.motion(&tree, 20.0, 10.0);
        assert!(input.contains_mouse(id));
        assert!(input.contains_mouse(handler));
        assert_eq!(input.cursor(), CursorShape::Pointer);
    }

    #[test]
    fn hidden_nodes_do_not_hit() {
        let (mut tree, mut input, id, chip) = setup();
        if let Some(node) = tree.get_mut(chip) {
            node.visible = false;
        }
        input.motion(&tree, 20.0, 10.0);
        assert!(!input.contains_mouse(id));
    }
}
