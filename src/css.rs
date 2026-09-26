use gtk::gdk;

pub const CSS: &str = r#"
@define-color shell_base #191724;
@define-color shell_text #e0def4;
@define-color shell_accent #c4a7e7;
@define-color shell_handle #9ccfd8;
* { font-family: "InputSans Nerd Font"; font-size: 12px; }
window#bar { background: @shell_base; border-bottom: 1px solid #403d52; }
.module { border: 0; box-shadow: none; min-height: 20px; min-width: 0; background: #1f1d2e; border-radius: 8px; padding: 4px 10px; margin: 4px 1px; color: @shell_text; }
.module:focus-visible { outline: 1px solid #c4a7e7; outline-offset: -2px; }
popover > contents { background: transparent; box-shadow: none; border: 0; padding: 0; }
.module:hover { background: #403d52; }
.workspaces { background: transparent; padding: 0; }
.workspace { min-height: 20px; box-shadow: none; background: transparent; color: #908caa; border: 0; border-radius: 8px; padding: 4px 9px; margin: 4px 1px; }
.workspace.empty { color: #6e6a86; }
.workspace.active { background: #26233a; color: #9ccfd8; }
.workspace.focused { background: #31748f; color: #191724; }
.workspace.urgent { color: #eb6f92; }
.clock { background: #1f1d2e; color: @shell_text; padding: 4px 8px; }
.audio { color: #ebbcba; min-width: 68px; }
.brightness, .temperature { color: #f6c177; }
.network, .language, .battery { color: #9ccfd8; }
.language, .battery { min-width: 68px; }
.battery.warning { color: #f6c177; }
.battery.critical { color: #eb6f92; }
.muted { color: #908caa; }
.tooltip { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 8px; padding: 6px 8px; }
window#launcher { background: @shell_base; border: 1px solid #403d52; border-radius: 12px; }
window#launcher label, window#launcher entry { font-size: 13pt; }
window#launcher .app-meta { font-size: 10pt; }
.launcher-box { padding: 14px; }
.search { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 8px; padding: 9px 12px; }
.search:focus { border-color: #c4a7e7; outline: none; box-shadow: none; }
window#launcher listbox row:focus { outline: none; }
.app-list { background: transparent; }
window#launcher listbox row { background: transparent; border-radius: 8px; }
window#launcher listbox row:hover { background: #26233a; }
window#launcher listbox row:selected { background: #26233a; color: @shell_text; }
window#launcher listbox row:selected label { color: @shell_text; }
window#launcher listbox row.selected-row { background-color: #26233a; color: @shell_text; }
window#launcher listbox row.selected-row label { color: @shell_text; }
.app-row { background-color: #1f1d2e; color: @shell_text; border-radius: 8px; padding: 8px 10px; }
window#launcher listbox row.selected-row .app-row { background-color: #26233a; }
.app-row.hidden-app { color: #908caa; }
.app-icon { margin-right: 12px; }
.app-meta { color: #6e6a86; font-size: 10px; }
.app-empty { color: #908caa; padding: 20px; }
window#notification { background: transparent; }
.notification-content { background: #1f1d2e; border: 1px solid #403d52; border-radius: 12px; padding: 14px 18px; color: @shell_text; }
.notification-icon { color: #9ccfd8; font-size: 24px; min-width: 32px; }
.notification-title { font-size: 13pt; font-weight: 600; }
.notification-detail { color: #908caa; font-size: 10pt; }
.notification-progress trough { background: #403d52; border-radius: 6px; min-height: 6px; }
.notification-progress progress { background: #9ccfd8; border-radius: 6px; min-height: 6px; }
.notification-toggle { color: #c4a7e7; }
.background-apps-toggle { color: #9ccfd8; }
window#background-apps-drawer { background: transparent; }
.background-apps-content { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 12px; padding: 14px; }
.background-apps-heading { font-size: 14px; font-weight: 600; }
.background-apps-empty { color: #908caa; padding: 18px; }
.background-app-row { background: #26233a; border-radius: 8px; padding: 8px; }
.background-app-action { background: #403d52; color: @shell_text; border-radius: 8px; padding: 5px 8px; }
.background-app-action:hover { background: #524f67; }
window#app-notification, window#notification-drawer { background: transparent; }
.app-notification-content, .notification-drawer-content { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 12px; padding: 14px; }
.app-notification-app { color: #9ccfd8; font-size: 10px; }
.app-notification-summary { color: @shell_text; font-weight: 600; font-size: 13px; }
.app-notification-body { color: #908caa; }
.notification-action, .notification-clear { background: #26233a; color: @shell_text; border-radius: 8px; padding: 6px 10px; }
.notification-action:hover, .notification-clear:hover { background: #403d52; }
.notification-drawer-heading { font-size: 14px; font-weight: 600; }
.notification-empty { color: #908caa; padding: 18px; }

.network-content { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 12px; padding: 14px; }
.network-heading { font-size: 14px; font-weight: 600; }
.network-section { color: #908caa; font-size: 10px; margin: 8px 2px 3px; }
.network-action { background: #26233a; color: @shell_text; border: 0; box-shadow: none; border-radius: 8px; padding: 7px 10px; }
.network-action:hover { background: #403d52; }
.network-action.primary, .network-action.enabled { background: #31748f; color: @shell_text; }
.network-action.primary:hover, .network-action.enabled:hover { background: #4086a0; }
.network-action.danger { color: #eb6f92; }
.network-content button:focus-visible { outline: 1px solid #c4a7e7; outline-offset: -2px; }
.network-content button:disabled { opacity: 0.5; }
.network-row { background: #26233a; color: @shell_text; border: 0; box-shadow: none; border-radius: 8px; padding: 10px; }
.network-row:hover { background: #403d52; }
.network-row.connected { background: #243541; }
.network-icon { color: #9ccfd8; font-size: 20px; min-width: 24px; }
.network-name { font-weight: 600; }
.network-meta, .network-status { color: #908caa; font-size: 11px; }
.network-signal { color: #9ccfd8; font-size: 11px; }
.network-empty { color: #908caa; padding: 18px 8px; }
.network-status.error { color: #eb6f92; }
.network-search, .network-password { background: @shell_base; color: @shell_text; border: 1px solid #403d52; border-radius: 8px; padding: 7px 10px; }
.network-search:focus-within, .network-password:focus-within { border-color: #c4a7e7; outline: none; box-shadow: none; }
.network-content dropdown > button { background: #26233a; color: @shell_text; border: 0; box-shadow: none; border-radius: 8px; }
.bar-edit-zone { background: #1f1d2e; color: #e0def4; border: 1px solid #403d52; border-radius: 8px; padding: 8px; }
.bar-insert { background: #26233a; color: #9ccfd8; border: 1px dashed #6e6a86; border-radius: 6px; min-width: 20px; padding: 4px; }
.bar-insert:drop(active), .bar-insert:hover { background: #403d52; border-color: #c4a7e7; }
window#layout-editor { background: transparent; }
window#chuh-menu { background: transparent; color: @shell_text; }
.menu-content { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 12px; padding: 18px; }
.menu-heading { font-size: 18px; font-weight: 600; }
.menu-back { padding: 5px 10px; min-height: 24px; }
.menu-list { background: transparent; color: @shell_text; }
.menu-list row { border: 0; box-shadow: none; border-radius: 8px; padding: 13px 14px; margin: 3px 0; background: #26233a; color: @shell_text; }
.menu-list row:selected, .menu-list row:hover { background: #403d52; color: @shell_text; }
.menu-list row:focus { outline: none; }
.menu-title { font-size: 15px; }
.menu-hint, .menu-wip .menu-title { color: #908caa; }
.menu-hint { font-size: 11px; }
.menu-state { color: #908caa; font-size: 13px; }
.menu-state.enabled { color: #9ccfd8; }
.menu-error { color: #eb6f92; }
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
