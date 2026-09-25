use gtk::gdk;

pub const CSS: &str = r#"
* { font-family: "InputSans Nerd Font"; font-size: 12px; }
window#bar { background: #191724; border-bottom: 1px solid #403d52; }
.module { background: #1f1d2e; border-radius: 8px; padding: 4px 10px; margin: 4px 1px; color: #e0def4; }
.module:hover { background: #403d52; }
.workspaces { background: transparent; padding: 0; }
.workspace { background: transparent; color: #908caa; border: 0; border-radius: 8px; padding: 4px 9px; margin: 4px 1px; }
.workspace.empty { color: #6e6a86; }
.workspace.active { background: #26233a; color: #9ccfd8; }
.workspace.focused { background: #31748f; color: #191724; }
.workspace.urgent { color: #eb6f92; }
.clock { background: #1f1d2e; color: #e0def4; padding: 4px 8px; }
.audio { color: #ebbcba; min-width: 68px; }
.brightness, .temperature { color: #f6c177; }
.network, .language, .battery { color: #9ccfd8; }
.language, .battery { min-width: 68px; }
.battery.warning { color: #f6c177; }
.battery.critical { color: #eb6f92; }
.muted { color: #908caa; }
.tooltip { background: #1f1d2e; color: #e0def4; border: 1px solid #403d52; border-radius: 8px; padding: 6px 8px; }
window#launcher { background: #191724; border: 1px solid #403d52; border-radius: 12px; }
window#launcher label, window#launcher entry { font-size: 13pt; }
window#launcher .app-meta { font-size: 10pt; }
.launcher-box { padding: 14px; }
.search { background: #1f1d2e; color: #e0def4; border: 1px solid #403d52; border-radius: 8px; padding: 9px 12px; }
.search:focus { border-color: #c4a7e7; outline: none; box-shadow: none; }
window#launcher listbox row:focus { outline: none; }
.app-list { background: transparent; }
window#launcher listbox row { background: transparent; border-radius: 8px; }
window#launcher listbox row:hover { background: #26233a; }
window#launcher listbox row:selected { background: #26233a; color: #e0def4; }
window#launcher listbox row:selected label { color: #e0def4; }
window#launcher listbox row.selected-row { background-color: #26233a; color: #e0def4; }
window#launcher listbox row.selected-row label { color: #e0def4; }
.app-row { background-color: #1f1d2e; color: #e0def4; border-radius: 8px; padding: 8px 10px; }
window#launcher listbox row.selected-row .app-row { background-color: #26233a; }
.app-row.hidden-app { color: #908caa; }
.app-icon { margin-right: 12px; }
.app-meta { color: #6e6a86; font-size: 10px; }
.app-empty { color: #908caa; padding: 20px; }
window#notification { background: transparent; }
.notification-content { background: #1f1d2e; border: 1px solid #403d52; border-radius: 12px; padding: 14px 18px; color: #e0def4; }
.notification-icon { color: #9ccfd8; font-size: 24px; min-width: 32px; }
.notification-title { font-size: 13pt; font-weight: 600; }
.notification-detail { color: #908caa; font-size: 10pt; }
.notification-progress trough { background: #403d52; border-radius: 6px; min-height: 6px; }
.notification-progress progress { background: #9ccfd8; border-radius: 6px; min-height: 6px; }
.notification-toggle { color: #c4a7e7; }
window#app-notification, window#notification-drawer { background: transparent; }
.app-notification-content, .notification-drawer-content { background: #1f1d2e; color: #e0def4; border: 1px solid #403d52; border-radius: 12px; padding: 14px; }
.app-notification-app { color: #9ccfd8; font-size: 10px; }
.app-notification-summary { color: #e0def4; font-weight: 600; font-size: 13px; }
.app-notification-body { color: #908caa; }
.notification-action, .notification-clear { background: #26233a; color: #e0def4; border-radius: 8px; padding: 6px 10px; }
.notification-action:hover, .notification-clear:hover { background: #403d52; }
.notification-drawer-heading { font-size: 14px; font-weight: 600; }
.notification-empty { color: #908caa; padding: 18px; }
"#;

pub fn install() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
