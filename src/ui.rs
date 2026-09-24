use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell;

const EDGES: [layer_shell::Edge; 4] = [
    layer_shell::Edge::Top,
    layer_shell::Edge::Bottom,
    layer_shell::Edge::Left,
    layer_shell::Edge::Right,
];

pub fn set_layer_window(
    window: &impl IsA<gtk::Window>,
    namespace: &str,
    layer: layer_shell::Layer,
    anchors: &[layer_shell::Edge],
    exclusive: i32,
    keyboard: layer_shell::KeyboardMode,
) {
    window.init_layer_shell();
    window.set_namespace(Some(namespace));
    window.set_layer(layer);
    for edge in EDGES {
        window.set_anchor(edge, anchors.contains(&edge));
    }
    window.set_exclusive_zone(exclusive);
    window.set_keyboard_mode(keyboard);
}
