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
window#launcher label, window#launcher entry { font-size: 14px; }
window#launcher .app-meta { font-size: 12px; }
.launcher-box { padding: 16px; }
.search { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 8px; padding: 9px 12px; }
.search:focus-within { border-color: #c4a7e7; outline: none; box-shadow: none; }
window#launcher .app-list row:focus { outline: none; }
.app-list { background: transparent; }
window#launcher .app-list row { background: transparent; border-radius: 8px; padding: 0; margin-bottom: 4px; }
window#launcher .app-list row:hover { background: #26233a; }
window#launcher .app-list row:selected { background: #26233a; color: @shell_text; }
window#launcher .app-list row:selected label { color: @shell_text; }
window#launcher .app-list row.selected-row { background-color: #26233a; color: @shell_text; }
window#launcher .app-list row.selected-row label { color: @shell_text; }
.app-row { background-color: #1f1d2e; color: @shell_text; border: 1px solid transparent; border-radius: 8px; padding: 10px 12px; min-height: 36px; }
window#launcher .app-list row.selected-row .app-row, window#launcher .app-list row:selected .app-row { background-color: #26233a; border-color: #6e6a86; box-shadow: inset 3px 0 @shell_accent; }
window#launcher button.app-pin, window#launcher button.launcher-sort { background: transparent; background-image: none; color: #908caa; border: 1px solid transparent; box-shadow: none; border-radius: 8px; padding: 6px 10px; min-height: 24px; }
window#launcher button.launcher-sort { background: #26233a; border-color: #403d52; color: @shell_text; }
window#launcher button.app-pin { margin-left: 12px; }
window#launcher .app-list row button.app-pin label { font-size: 18px; color: #908caa; }
window#launcher button.app-pin:hover, window#launcher button.launcher-sort:hover { background: #403d52; color: @shell_text; }
window#launcher .app-list row button.app-pin.pinned label { color: @shell_accent; }
window#launcher button.app-pin:focus, window#launcher button.launcher-sort:focus { background: #403d52; outline: 1px solid @shell_accent; outline-offset: -1px; }
.app-row.hidden-app { color: #908caa; }
.app-icon { margin-right: 12px; min-width: 28px; }
.app-meta { color: #908caa; font-size: 11px; }
.app-empty { color: #908caa; padding: 20px; }
window#notification { background: transparent; }
.notification-content { background: #1f1d2e; border: 1px solid #403d52; border-radius: 12px; padding: 14px 18px; color: @shell_text; }
.notification-icon { color: #9ccfd8; font-size: 24px; min-width: 32px; }
.notification-title { font-size: 16px; font-weight: 600; }
.notification-detail { color: #908caa; font-size: 12px; }
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
.app-notification-app { color: #9ccfd8; font-size: 11px; font-weight: 500; }
.app-notification-summary { color: @shell_text; font-weight: 600; font-size: 13px; }
.app-notification-body { color: #908caa; }
.notification-action, .notification-clear { background: #26233a; color: @shell_text; border-radius: 8px; padding: 6px 10px; }
.notification-action:hover, .notification-clear:hover { background: #403d52; }
.notification-drawer-heading { font-size: 14px; font-weight: 600; }
.notification-empty { color: #908caa; padding: 18px; }

.keybinding-row { color: @shell_text; padding: 0; }
.keybinding-row .network-action, .bluetooth-device .network-action { background: #1f1d2e; }
.keybinding-row .network-action:hover, .bluetooth-device .network-action:hover { background: #403d52; }
.keybinding-editor { background: #26233a; border-radius: 8px; padding: 10px; }
.network-content { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 12px; padding: 14px; }
.network-heading { font-size: 14px; font-weight: 600; }
.network-section { color: #908caa; font-size: 11px; font-weight: 600; margin: 8px 2px 4px; }
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
.menu-content dropdown > button, .network-content dropdown > button { background: #26233a; color: @shell_text; border: 0; box-shadow: none; border-radius: 8px; }
.bar-edit-zone { background: #1f1d2e; color: #e0def4; border: 1px solid #403d52; border-radius: 8px; padding: 8px; }
.bar-insert { background: #26233a; color: #9ccfd8; border: 1px dashed #6e6a86; border-radius: 6px; min-width: 20px; padding: 4px; }
.bar-insert:drop(active), .bar-insert:hover { background: #403d52; border-color: #c4a7e7; }
window#layout-editor { background: transparent; }
window#chuh-menu { background: transparent; color: @shell_text; }
.menu-content { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 12px; padding: 18px; }
.menu-heading { font-size: 18px; font-weight: 600; }
.menu-back { padding: 5px 10px; min-height: 24px; }
.menu-list { background: transparent; color: @shell_text; }
.menu-list row { border: 1px solid transparent; box-shadow: none; border-radius: 8px; padding: 12px; margin: 3px 0; background: #26233a; color: @shell_text; }
.menu-list row:selected, .menu-list row:hover { background: #403d52; color: @shell_text; }
.menu-list row:focus { outline: none; }
.menu-title { font-size: 14px; font-weight: 500; }
.menu-hint, .menu-wip .menu-title { color: #908caa; }
.menu-hint { font-size: 12px; }
.menu-state { color: #908caa; font-size: 13px; }
.menu-state.enabled { color: #9ccfd8; }
.menu-error { color: #eb6f92; }
.calendar-weekday { color: #908caa; padding: 6px 0; }
.calendar-day { min-width: 28px; min-height: 26px; padding: 6px; }
.calendar-day.outside { color: #6e6a86; background: transparent; }
.calendar-day.today { background: #31748f; color: #e0def4; font-weight: 600; }
.calendar-day:focus-visible { outline: 1px solid #c4a7e7; }
.todo-entry { background: #26233a; color: @shell_text; border: 1px solid #403d52; border-radius: 8px; padding: 8px 10px; }
.todo-entry:focus-within { border-color: #c4a7e7; outline: none; box-shadow: none; }
.todo-row { background: #26233a; border-radius: 8px; padding: 8px 10px; }
.todo-row checkbutton { color: #9ccfd8; }
.todo-done { color: #908caa; text-decoration: line-through; }
.todo-delete { background: transparent; color: #908caa; border: 0; box-shadow: none; min-width: 28px; min-height: 28px; padding: 0; }
.todo-delete:hover { background: #403d52; color: #eb6f92; }
.todo-empty { background: #26233a; border-radius: 8px; padding: 18px; }
.clipboard-preview { color: @shell_text; }
.weather-report { color: @shell_text; }
.weather-current { background: #26233a; border-radius: 10px; padding: 16px; }
.weather-temperature { font-size: 36px; font-weight: 600; color: #f6c177; }
.weather-day { background: #26233a; border-radius: 8px; padding: 10px 12px; }
.weather-range { color: #9ccfd8; }
.clipboard-image { color: #9ccfd8; font-size: 32px; min-width: 48px; }
.bluetooth-device { background: #26233a; border-radius: 8px; padding: 12px; }
.bluetooth-prompt { color: #9ccfd8; }
button, entry, searchentry, dropdown > button, row, .app-row, .menu-state, scrollbar slider, progress {
    text-shadow: none;
    transition: background-color 160ms ease-out, border-color 160ms ease-out, color 160ms ease-out, box-shadow 180ms ease-out, opacity 160ms ease-out, transform 140ms ease-out;
}
button:focus-visible, row:focus-visible, dropdown:focus-visible {
    outline: 1px solid @shell_accent;
    outline-offset: -3px;
}
button:disabled { opacity: 0.5; }
button:active { background-image: none; }
.module, .workspace { font-weight: 500; }
.workspace:hover { background: #26233a; }
.workspace.focused:hover { background: #31748f; color: @shell_base; }
.module-disabled { opacity: 0.45; }
tooltip { background: #1f1d2e; color: @shell_text; border: 1px solid #403d52; border-radius: 8px; padding: 6px 8px; }
.search, .network-search, .network-password { min-height: 24px; caret-color: @shell_accent; }
.search image { color: #908caa; }
entry selection, text selection { background: #403d52; color: @shell_text; }
window#launcher .app-list row:hover .app-row { background-color: #26233a; }
window#launcher .app-list row:selected .app-meta,
window#launcher .app-list row.selected-row .app-meta { color: #908caa; }
.app-name { font-weight: 500; }
.menu-header { padding-bottom: 12px; border-bottom: 1px solid #403d52; margin-bottom: 2px; }
.menu-label, .network-name { font-weight: 500; }
.menu-state { min-width: 28px; }
.menu-list row:selected { border-color: #6e6a86; }
.menu-list row:focus-visible { outline: 1px solid @shell_accent; outline-offset: -3px; }
.menu-list row:disabled { opacity: 0.5; }
.network-content, .background-apps-content, .notification-drawer-content { padding: 16px; }
.network-heading, .background-apps-heading, .notification-drawer-heading { font-size: 16px; }
.network-row, .bluetooth-device, .background-app-row { padding: 12px; }
.network-row.connected { box-shadow: inset 3px 0 #9ccfd8; }
.background-app-action, .notification-action, .notification-clear {
    border: 1px solid transparent;
    box-shadow: none;
    background-image: none;
    min-height: 24px;
}
.network-action { min-height: 24px; }
.network-action:active, .notification-action:active, .notification-clear:active,
.background-app-action:active { background: #524f67; }
.menu-content dropdown > button, .network-content dropdown > button { padding: 7px 10px; min-height: 24px; }
dropdown popover > contents {
    background: #1f1d2e;
    color: @shell_text;
    border: 1px solid #403d52;
    border-radius: 8px;
    padding: 6px;
}
dropdown popover listview { background: transparent; color: @shell_text; }
dropdown popover row { padding: 8px 10px; border-radius: 6px; }
dropdown popover row:hover, dropdown popover row:selected { background: #403d52; }
.control-scale trough { background: #403d52; border: 0; border-radius: 6px; min-height: 6px; }
.control-scale highlight { background: @shell_handle; border: 0; border-radius: 6px; min-width: 0; min-height: 6px; margin: 0; padding: 0; }
.control-scale slider { background: @shell_handle; border: 1px solid #403d52; border-radius: 50%; min-width: 14px; min-height: 14px; margin: 0; padding: 0; box-shadow: none; }
.control-scale value { color: @shell_text; }
separator { background: #403d52; min-height: 1px; min-width: 1px; }
scrollbar { background: transparent; }
scrollbar slider { background: #403d52; border: 0; border-radius: 8px; min-width: 4px; min-height: 4px; }
scrollbar slider:hover { background: #6e6a86; }
scrollbar.vertical { margin-left: 6px; }
.notification-progress trough, .notification-progress progress { border: 0; }
.app-notification-summary { font-size: 14px; }
.app-notification-body { font-size: 12px; }
.keybinding-editor { padding: 12px; border: 1px solid #403d52; }
.calendar-weekday { font-size: 11px; font-weight: 600; }
.calendar-day { border-radius: 8px; }
.weather-day { padding: 12px; }
.bar-edit-zone { padding: 12px; }
@keyframes shell-disappear {
    from { opacity: 1; }
    to { opacity: 0; }
}
window.shell-closing, window#launcher.shell-closing { animation: shell-disappear 140ms ease-out forwards; }
@keyframes shell-appear {
    from { opacity: 0; transform: scale(0.97); }
    to { opacity: 1; transform: scale(1); }
}
@keyframes shell-rise {
    from { opacity: 0; transform: translateY(10px); }
    to { opacity: 1; transform: translateY(0); }
}
@keyframes shell-unfold {
    from { opacity: 0; transform: scaleY(0.88); }
    to { opacity: 1; transform: scaleY(1); }
}
window#launcher, window#chuh-menu > .menu-content,
window#layout-editor .menu-content, .keybinding-editor {
    animation: shell-appear 200ms cubic-bezier(0.16, 1, 0.3, 1);
}
.notification-content, window#app-notification .app-notification-content {
    animation: shell-rise 220ms cubic-bezier(0.16, 1, 0.3, 1);
}
popover.shell-popover > contents {
    transform-origin: top center;
    animation: shell-unfold 220ms cubic-bezier(0.16, 1, 0.3, 1);
}
button:active { transform: scale(0.97); }
.module:active, .workspace:active { transform: none; }
.module.popup-open { background: #26233a; box-shadow: inset 0 -2px @shell_handle; }
.module.network.popup-open {
    background: @shell_base;
    border-radius: 8px 8px 0 0;
    box-shadow: 0 5px @shell_base, inset 0 2px @shell_handle;
}
popover.bar-attached, popover.bar-attached > contents { margin: 0; padding: 0; }
popover.bar-attached .network-content {
    background: @shell_base;
    border-top: 0;
    border-radius: 0 0 12px 12px;
    min-width: 340px;
}
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
