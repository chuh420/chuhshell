use std::cell::{Cell, RefCell};

use crate::apps::AppEntry;
use crate::niri::Workspace;
use crate::notifications::OsdWidgets;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LauncherMode {
    Normal,
    Manage,
}

#[derive(Default)]
pub struct AppState {
    pub bar: RefCell<Option<gtk::Window>>,
    pub launcher: RefCell<Option<gtk::Window>>,
    pub osd: RefCell<Option<gtk::Window>>,
    pub osd_widgets: RefCell<Option<OsdWidgets>>,
    pub osd_timeout: RefCell<Option<glib::SourceId>>,
    pub osd_generation: Cell<u64>,
    pub workspaces: RefCell<Vec<Workspace>>,
    pub layout_names: RefCell<Vec<String>>,
    pub current_layout: Cell<usize>,
    pub launcher_mode: Cell<Option<LauncherMode>>,
    pub launcher_apps: RefCell<Vec<AppEntry>>,
    pub launcher_focus_window: Cell<Option<Option<u64>>>,
}

impl AppState {
    pub fn focused_window_id(&self) -> Option<u64> {
        self.workspaces
            .borrow()
            .iter()
            .find(|workspace| workspace.is_focused)
            .and_then(|workspace| workspace.active_window_id)
    }
}
