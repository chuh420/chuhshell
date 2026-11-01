use std::sync::Arc;

use platform::{Damage, Rect};
use tiny_skia::{Pixmap, Transform};

use crate::{
    Color,
    text::{Raster, TextSpec},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformOp {
    Translate {
        x: f32,
        y: f32,
    },
    Scale {
        x: f32,
        y: f32,
        origin_x: f32,
        origin_y: f32,
    },
    Rotate {
        angle: f32,
        origin_x: f32,
        origin_y: f32,
    },
}

impl TransformOp {
    fn matrix(self) -> Transform {
        match self {
            TransformOp::Translate { x, y } => Transform::from_translate(x, y),
            TransformOp::Scale {
                x,
                y,
                origin_x,
                origin_y,
            } => Transform::from_translate(origin_x, origin_y)
                .pre_scale(x, y)
                .pre_translate(-origin_x, -origin_y),
            TransformOp::Rotate {
                angle,
                origin_x,
                origin_y,
            } => Transform::from_translate(origin_x, origin_y)
                .pre_concat(Transform::from_rotate(angle))
                .pre_translate(-origin_x, -origin_y),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectPaint {
    pub color: Color,
    pub radius: f32,
    pub border_color: Color,
    pub border_width: f32,
}

pub struct TextPaint {
    spec: TextSpec,
    pub(crate) raster: Option<Raster>,
}

impl TextPaint {
    pub fn new(spec: TextSpec) -> Self {
        Self { spec, raster: None }
    }

    pub fn spec(&self) -> &TextSpec {
        &self.spec
    }

    pub fn set_spec(&mut self, spec: TextSpec) -> bool {
        if self.spec == spec {
            return false;
        }
        self.spec = spec;
        self.raster = None;
        true
    }

    fn overflow(&self) -> f32 {
        self.spec
            .spans
            .iter()
            .map(|span| span.size)
            .fold(0.0, f32::max)
            * 0.5
            + 2.0
    }
}

pub struct SvgPaint {
    pub(crate) tree: Arc<resvg::usvg::Tree>,
    pub(crate) raster: Option<Pixmap>,
}

impl SvgPaint {
    pub fn new(tree: Arc<resvg::usvg::Tree>) -> Self {
        Self { tree, raster: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathPaint {
    pub points: Vec<(f32, f32)>,
    pub stroke: Color,
    pub width: f32,
    pub fill: Color,
    pub fill_to: Option<f32>,
}

#[derive(Default)]
pub enum Paint {
    #[default]
    None,
    Rect(RectPaint),
    Text(TextPaint),
    Svg(SvgPaint),
    Image(Arc<Pixmap>),
    Path(PathPaint),
}

pub struct Node {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
    pub visible: bool,
    pub scale: f32,
    pub rotation: f32,
    pub origin: Option<(f32, f32)>,
    pub transforms: Vec<TransformOp>,
    pub clip: bool,
    pub paint: Paint,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
    dirty: bool,
    bounds: Option<Rect>,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            opacity: 1.0,
            visible: true,
            scale: 1.0,
            rotation: 0.0,
            origin: None,
            transforms: Vec::new(),
            clip: false,
            paint: Paint::None,
            children: Vec::new(),
            parent: None,
            dirty: true,
            bounds: None,
        }
    }
}

impl Node {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            ..Self::default()
        }
    }

    pub fn with_paint(mut self, paint: Paint) -> Self {
        self.paint = paint;
        self
    }

    pub fn rect(x: f32, y: f32, width: f32, height: f32, paint: RectPaint) -> Self {
        Self::new(x, y, width, height).with_paint(Paint::Rect(paint))
    }

    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    pub fn local_transform(&self) -> Transform {
        let mut matrix = Transform::from_translate(self.x, self.y);
        for op in self.transforms.iter().rev() {
            matrix = matrix.pre_concat(op.matrix());
        }
        if self.scale != 1.0 || self.rotation != 0.0 {
            let (ox, oy) = self.origin.unwrap_or((self.width / 2.0, self.height / 2.0));
            matrix = matrix
                .pre_translate(ox, oy)
                .pre_scale(self.scale, self.scale)
                .pre_concat(Transform::from_rotate(self.rotation))
                .pre_translate(-ox, -oy);
        }
        matrix
    }

    pub(crate) fn overflow(&self) -> f32 {
        match &self.paint {
            Paint::Text(text) => text.overflow(),
            _ => 0.0,
        }
    }
}

pub struct Tree {
    nodes: Vec<Option<Node>>,
    free: Vec<usize>,
    root: NodeId,
    removed: Damage,
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    pub fn new() -> Self {
        Self {
            nodes: vec![Some(Node::default())],
            free: Vec::new(),
            root: NodeId(0),
            removed: Damage::default(),
        }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn add(&mut self, parent: NodeId, mut node: Node) -> NodeId {
        node.parent = Some(parent);
        node.dirty = true;
        let id = match self.free.pop() {
            Some(index) => {
                self.nodes[index] = Some(node);
                NodeId(index)
            }
            None => {
                self.nodes.push(Some(node));
                NodeId(self.nodes.len() - 1)
            }
        };
        if let Some(parent) = self.nodes.get_mut(parent.0).and_then(Option::as_mut) {
            parent.children.push(id);
        }
        id
    }

    pub fn remove(&mut self, id: NodeId) {
        if id == self.root {
            return;
        }
        let Some(node) = self.nodes.get_mut(id.0).and_then(Option::take) else {
            return;
        };
        if let Some(bounds) = node.bounds {
            self.removed.add(bounds);
        }
        if let Some(parent) = node.parent.and_then(|p| self.nodes.get_mut(p.0)?.as_mut()) {
            parent.children.retain(|child| *child != id);
        }
        self.free.push(id.0);
        for child in node.children {
            self.remove(child);
        }
    }

    pub fn clear_children(&mut self, id: NodeId) {
        let children = self
            .get(id)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child in children {
            self.remove(child);
        }
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0).and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let node = self.nodes.get_mut(id.0).and_then(Option::as_mut)?;
        node.dirty = true;
        Some(node)
    }

    pub(crate) fn get_mut_clean(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0).and_then(Option::as_mut)
    }

    pub fn device_transform(&self, id: NodeId) -> Transform {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(node_id) = current {
            let Some(node) = self.get(node_id) else {
                break;
            };
            chain.push(node.local_transform());
            current = node.parent;
        }
        chain
            .into_iter()
            .rev()
            .fold(Transform::identity(), |acc, local| acc.pre_concat(local))
    }

    fn update<F: FnOnce(&mut Node) -> bool>(&mut self, id: NodeId, f: F) {
        if let Some(node) = self.get_mut_clean(id)
            && f(node)
        {
            node.dirty = true;
        }
    }

    pub fn set_pos(&mut self, id: NodeId, x: f32, y: f32) {
        self.update(id, |node| {
            let changed = node.x != x || node.y != y;
            node.x = x;
            node.y = y;
            changed
        });
    }

    pub fn set_size(&mut self, id: NodeId, width: f32, height: f32) {
        self.update(id, |node| {
            let changed = node.width != width || node.height != height;
            node.width = width;
            node.height = height;
            changed
        });
    }

    pub fn set_opacity(&mut self, id: NodeId, opacity: f32) {
        self.update(id, |node| {
            let changed = node.opacity != opacity;
            node.opacity = opacity;
            changed
        });
    }

    pub fn set_visible(&mut self, id: NodeId, visible: bool) {
        self.update(id, |node| {
            let changed = node.visible != visible;
            node.visible = visible;
            changed
        });
    }

    pub fn set_scale(&mut self, id: NodeId, scale: f32) {
        self.update(id, |node| {
            let changed = node.scale != scale;
            node.scale = scale;
            changed
        });
    }

    pub fn set_transforms(&mut self, id: NodeId, transforms: &[TransformOp]) {
        self.update(id, |node| {
            let changed = node.transforms != transforms;
            if changed {
                node.transforms = transforms.to_vec();
            }
            changed
        });
    }

    pub fn set_fill(&mut self, id: NodeId, color: Color) {
        self.update(id, |node| match &mut node.paint {
            Paint::Rect(rect) if rect.color != color => {
                rect.color = color;
                true
            }
            _ => false,
        });
    }

    pub fn set_text(&mut self, id: NodeId, spec: TextSpec) {
        self.update(id, |node| match &mut node.paint {
            Paint::Text(text) => text.set_spec(spec),
            _ => false,
        });
    }

    pub fn text_spec(&self, id: NodeId) -> Option<&TextSpec> {
        match &self.get(id)?.paint {
            Paint::Text(text) => Some(text.spec()),
            _ => None,
        }
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.get(id).and_then(|node| node.parent)
    }

    pub fn is_ancestor_or_self(&self, ancestor: NodeId, id: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            if node == ancestor {
                return true;
            }
            current = self.parent(node);
        }
        false
    }

    pub fn paint_order(&self) -> Vec<NodeId> {
        let mut order = Vec::new();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            order.push(id);
            if let Some(node) = self.get(id) {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        order
    }

    pub fn hit(&self, id: NodeId, x: f32, y: f32) -> bool {
        let mut current = Some(id);
        while let Some(node_id) = current {
            let Some(node) = self.get(node_id) else {
                return false;
            };
            if !node.visible {
                return false;
            }
            if (node_id == id || node.clip) && !self.local_contains(node_id, x, y) {
                return false;
            }
            current = node.parent;
        }
        true
    }

    fn local_contains(&self, id: NodeId, x: f32, y: f32) -> bool {
        let Some(node) = self.get(id) else {
            return false;
        };
        let Some(inverse) = self.device_transform(id).invert() else {
            return false;
        };
        let mut point = tiny_skia::Point::from_xy(x, y);
        inverse.map_point(&mut point);
        point.x >= 0.0 && point.y >= 0.0 && point.x < node.width && point.y < node.height
    }

    pub fn mark_all(&mut self) {
        for node in self.nodes.iter_mut().flatten() {
            node.dirty = true;
        }
    }

    pub fn collect_damage(&mut self) -> Damage {
        let mut damage = std::mem::take(&mut self.removed);
        self.collect(self.root, Transform::identity(), true, false, &mut damage);
        damage
    }

    fn collect(
        &mut self,
        id: NodeId,
        parent: Transform,
        parent_visible: bool,
        parent_dirty: bool,
        damage: &mut Damage,
    ) {
        let Some(node) = self.get_mut_clean(id) else {
            return;
        };
        let dirty = node.dirty || parent_dirty;
        let visible = parent_visible && node.visible && node.opacity > 0.0;
        let transform = parent.pre_concat(node.local_transform());
        if dirty {
            let bounds = if visible {
                device_bounds(transform, node.width, node.height, node.overflow())
            } else {
                None
            };
            if let Some(old) = node.bounds {
                damage.add(old);
            }
            if let Some(new) = bounds {
                damage.add(new);
            }
            node.bounds = bounds;
            node.dirty = false;
        }
        let children = node.children.clone();
        for child in children {
            self.collect(child, transform, visible, dirty, damage);
        }
    }
}

pub(crate) fn device_bounds(
    transform: Transform,
    width: f32,
    height: f32,
    overflow: f32,
) -> Option<Rect> {
    let corners = [
        (-overflow, -overflow),
        (width + overflow, -overflow),
        (-overflow, height + overflow),
        (width + overflow, height + overflow),
    ];
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for (x, y) in corners {
        let mut point = tiny_skia::Point::from_xy(x, y);
        transform.map_point(&mut point);
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) {
        return None;
    }
    let x = min_x.floor() as i32 - 1;
    let y = min_y.floor() as i32 - 1;
    let rect = Rect::new(
        x,
        y,
        max_x.ceil() as i32 + 1 - x,
        max_y.ceil() as i32 + 1 - y,
    );
    (!rect.is_empty()).then_some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_node_damages_old_and_new() {
        let mut tree = Tree::new();
        let root = tree.root();
        let id = tree.add(root, Node::new(0.0, 0.0, 10.0, 10.0));
        tree.collect_damage();
        if let Some(node) = tree.get_mut(id) {
            node.x = 100.0;
        }
        let damage = tree.collect_damage();
        assert_eq!(damage.rects().len(), 2);
    }

    #[test]
    fn clean_tree_has_no_damage() {
        let mut tree = Tree::new();
        let root = tree.root();
        tree.add(root, Node::new(0.0, 0.0, 10.0, 10.0));
        tree.collect_damage();
        assert!(tree.collect_damage().is_empty());
    }

    #[test]
    fn removed_node_damages_its_bounds() {
        let mut tree = Tree::new();
        let root = tree.root();
        let id = tree.add(root, Node::new(5.0, 5.0, 10.0, 10.0));
        tree.collect_damage();
        tree.remove(id);
        let damage = tree.collect_damage();
        assert_eq!(damage.rects(), &[Rect::new(4, 4, 12, 12)]);
    }

    #[test]
    fn scale_about_top_left() {
        let mut node = Node::new(0.0, 0.0, 100.0, 10.0);
        node.scale = 2.0;
        node.origin = Some((0.0, 0.0));
        let mut point = tiny_skia::Point::from_xy(10.0, 10.0);
        node.local_transform().map_point(&mut point);
        assert_eq!((point.x, point.y), (20.0, 20.0));
    }
}
