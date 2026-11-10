use platform::{KeyEvent, MouseButton};
use services::vpn::{self, Country, Target};
use ui::{
    Color, Node, NodeId, TextEngine, Tree,
    anim::Millis,
    input::{AreaId, Input, InputEvent, MouseArea},
    layout::{Cell, Slot, center, linear},
    textfield::{KeyResult, TextField},
    widgets::{Chip, Label, Sparkline},
};

use super::{
    PanelAction, PanelData, TooltipSpot, VpnAction,
    kit::{Scroller, chip, fit, icon_label, label, place_chip, place_label, rect, set_border},
    wifi::speed,
};
use crate::{
    theme::{self},
    vpnctl::{VpnData, VpnState, bytes, duration},
};

const ROW: f32 = 52.0;
const SETTINGS_SHOWN: [&str; 3] = ["netshield", "kill-switch", "vpn-accelerator"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    All,
    Favorites,
    Recent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Name,
    Speed,
}

#[derive(Clone, PartialEq)]
struct RowKey {
    code: String,
    sub: String,
    current: bool,
    favorite: bool,
    usable: bool,
}

struct Row {
    key: RowKey,
    root: NodeId,
    hit: NodeId,
    area: AreaId,
    badge: NodeId,
    code: Label,
    name: Label,
    sub: Label,
    star: Chip,
}

struct SettingChip {
    key: &'static str,
    chip: Chip,
}

fn setting_title(key: &str) -> &'static str {
    match key {
        "netshield" => "netshield",
        "kill-switch" => "kill switch",
        "vpn-accelerator" => "accel",
        _ => "setting",
    }
}

fn setting_value(value: &str) -> &str {
    match value {
        "malware-only" => "malware",
        "malware-ads-trackers" => "ads + malware",
        "" => "…",
        other => other,
    }
}

fn setting_help(key: &str, value: &str) -> String {
    let what = match key {
        "netshield" => "NetShield blocks malware (and ads with trackers) through DNS",
        "kill-switch" => "Kill switch blocks the internet if the VPN drops",
        "vpn-accelerator" => "VPN Accelerator speeds up distant connections",
        _ => "",
    };
    let next = vpn::next_value(key, value).unwrap_or("");
    format!("{what}\nnow: {value} · click: {next}\napplies on the next connection")
}

fn load_color(load: u32) -> Color {
    if load >= 80 {
        theme::red()
    } else if load >= 55 {
        theme::yellow()
    } else {
        theme::green()
    }
}

fn country_sub(country: &Country, data: &VpnData, current: bool) -> String {
    let mut parts = Vec::new();
    if current {
        parts.push("connected · click to disconnect".to_string());
    } else if data.home.as_deref() == Some(country.code.as_str()) {
        parts.push("your location".to_string());
    } else if data.exclude.contains(&country.code) {
        parts.push("not used for fastest".to_string());
    }
    if country.online == 0 {
        parts.push("not on your plan".into());
        return parts.join(" · ");
    }
    parts.push(format!(
        "{} server{}",
        country.online,
        if country.online == 1 { "" } else { "s" }
    ));
    if country.cities > 1 {
        parts.push(format!("{} cities", country.cities));
    }
    parts.push(format!("load {}%", country.load));
    let mut tags = Vec::new();
    if country.p2p {
        tags.push("p2p");
    }
    if country.streaming {
        tags.push("streaming");
    }
    if country.secure_core {
        tags.push("sc");
    }
    if country.tor {
        tags.push("tor");
    }
    if !tags.is_empty() {
        parts.push(tags.join(" "));
    }
    parts.join(" · ")
}

pub struct VpnPanel {
    pub root: NodeId,
    title: Label,
    refresh: Chip,
    power: Chip,
    status: Label,
    card: NodeId,
    card_icon: Label,
    place: Label,
    load: Label,
    server: Label,
    uptime: Label,
    ip: Label,
    copy: Chip,
    spark: Sparkline,
    down: Label,
    up: Label,
    total: Label,
    quick: Vec<(Chip, Target)>,
    reconnect: Chip,
    settings: Vec<SettingChip>,
    all: Chip,
    favorites: Chip,
    recent: Chip,
    sort_chip: Chip,
    search_frame: NodeId,
    search: TextField,
    placeholder: Label,
    list: Scroller,
    rows: Vec<Row>,
    empty: Label,
    message: Label,
    section: Section,
    sort: Sort,
    data: PanelData,
}

impl VpnPanel {
    pub fn new(
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        parent: NodeId,
        data: &PanelData,
        now: Millis,
    ) -> Self {
        let root = tree.add(parent, Node::new(0.0, 0.0, 0.0, 0.0));
        let title = label(tree, engine, root, "Proton VPN", 18.0, theme::text());
        let mut refresh = chip(tree, input, engine, root, "󰑐", theme::green(), false, None);
        refresh.shortcut = "update the server list and settings".into();
        let power = chip(
            tree,
            input,
            engine,
            root,
            "connect",
            theme::green(),
            false,
            None,
        );
        let status = label(tree, engine, root, "", 12.0, theme::muted());
        let card = rect(tree, root, theme::surface(), 14.0);
        let card_icon = icon_label(tree, engine, card, "󰒙", 18.0, theme::muted());
        let place = label(tree, engine, card, "", 13.0, theme::text());
        let load = label(tree, engine, card, "", 11.0, theme::green());
        let server = label(tree, engine, card, "", 10.0, theme::muted());
        let uptime = icon_label(tree, engine, card, "", 10.0, theme::text());
        let ip = label(tree, engine, card, "", 10.0, theme::muted());
        let mut copy = chip(tree, input, engine, card, "󰆏", theme::muted(), true, None);
        copy.shortcut = "copy the server IP address".into();
        let spark = Sparkline::new(tree, card, theme::green(), theme::blue(), theme::line());
        let down = icon_label(tree, engine, card, "", 10.0, theme::green());
        let up = icon_label(tree, engine, card, "", 10.0, theme::blue());
        let total = label(tree, engine, card, "", 10.0, theme::muted());
        let quick_items = [
            (
                "random",
                Target::Random,
                "connect to a random server on your plan",
            ),
            ("p2p", Target::P2p, "fastest server that allows P2P"),
            (
                "secure core",
                Target::SecureCore,
                "route through a hardened Secure Core server first",
            ),
            ("tor", Target::Tor, "fastest Tor over VPN server"),
        ];
        let quick = quick_items
            .into_iter()
            .map(|(text, target, help)| {
                let mut item = chip(tree, input, engine, root, text, theme::text(), true, None);
                item.set_font_size(engine, 11.0, 12.0);
                item.shortcut = help.into();
                (item, target)
            })
            .collect();
        let mut reconnect = chip(
            tree,
            input,
            engine,
            root,
            "reconnect",
            theme::text(),
            true,
            None,
        );
        reconnect.set_font_size(engine, 11.0, 12.0);
        reconnect.shortcut = "pick the fastest server in the same country again".into();
        let settings = SETTINGS_SHOWN
            .iter()
            .map(|key| {
                let mut item = chip(tree, input, engine, root, "", theme::muted(), true, None);
                item.set_font_size(engine, 11.0, 12.0);
                SettingChip { key, chip: item }
            })
            .collect();
        let all = chip(tree, input, engine, root, "all", theme::text(), false, None);
        let favorites = chip(
            tree,
            input,
            engine,
            root,
            "󰓎 favorites",
            theme::text(),
            false,
            None,
        );
        let recent = chip(
            tree,
            input,
            engine,
            root,
            "󰋚 recent",
            theme::text(),
            false,
            None,
        );
        let mut sort_chip = chip(tree, input, engine, root, "a–z", theme::text(), false, None);
        sort_chip.shortcut = "sort by name or by speed".into();
        let search_frame = rect(tree, root, theme::surface(), 8.0);
        set_border(tree, search_frame, theme::line(), 1.0);
        let placeholder = label(
            tree,
            engine,
            search_frame,
            "search countries · enter connects",
            12.0,
            theme::muted(),
        );
        let search = TextField::new(
            tree,
            input,
            engine,
            search_frame,
            crate::theme::font(),
            12.0,
            theme::text(),
            false,
        );
        let list = Scroller::new(tree, input, root, true);
        let empty = label(tree, engine, list.view, "", 12.0, theme::muted());
        let mut message = label(tree, engine, root, "", 11.0, theme::red());
        message.set_color(theme::red());
        let mut panel = Self {
            root,
            title,
            refresh,
            power,
            status,
            card,
            card_icon,
            place,
            load,
            server,
            uptime,
            ip,
            copy,
            spark,
            down,
            up,
            total,
            quick,
            reconnect,
            settings,
            all,
            favorites,
            recent,
            sort_chip,
            search_frame,
            search,
            placeholder,
            list,
            rows: Vec::new(),
            empty,
            message,
            section: Section::All,
            sort: Sort::Name,
            data: PanelData::default(),
        };
        panel.search.set_focus(true, now);
        panel.sync(tree, input, engine, data, now);
        panel
    }

    pub fn areas(&self) -> Vec<AreaId> {
        let mut areas = vec![
            self.refresh.area,
            self.power.area,
            self.copy.area,
            self.reconnect.area,
            self.all.area,
            self.favorites.area,
            self.recent.area,
            self.sort_chip.area,
            self.search.area,
        ];
        areas.extend(self.quick.iter().map(|(chip, _)| chip.area));
        areas.extend(self.settings.iter().map(|item| item.chip.area));
        areas.extend(self.list.areas());
        for row in &self.rows {
            areas.extend([row.area, row.star.area]);
        }
        areas
    }

    fn vpn(&self) -> &VpnData {
        &self.data.vpn
    }

    fn current_country(&self) -> Option<String> {
        let vpn = self.vpn();
        (vpn.state == VpnState::On).then(|| vpn.country()).flatten()
    }

    fn filtered(&self) -> Vec<Country> {
        let vpn = self.vpn();
        let Some(catalog) = &vpn.catalog else {
            return Vec::new();
        };
        let query = self.search.text().trim().to_lowercase();
        let matches = |country: &Country| {
            query.is_empty()
                || country.name.to_lowercase().contains(&query)
                || country.code.to_lowercase() == query
        };
        let mut countries: Vec<Country> = match self.section {
            Section::All => catalog
                .countries
                .iter()
                .filter(|c| matches(c))
                .cloned()
                .collect(),
            Section::Favorites | Section::Recent => {
                let codes = if self.section == Section::Favorites {
                    &vpn.favorites
                } else {
                    &vpn.recent
                };
                codes
                    .iter()
                    .filter_map(|code| catalog.countries.iter().find(|c| c.code == *code))
                    .filter(|c| matches(c))
                    .cloned()
                    .collect()
            }
        };
        if self.sort == Sort::Speed && self.section != Section::Recent {
            countries.sort_by(|a, b| {
                a.score
                    .unwrap_or(f64::MAX)
                    .total_cmp(&b.score.unwrap_or(f64::MAX))
            });
        }
        countries
    }

    fn rebuild(&mut self, tree: &mut Tree, input: &mut Input, engine: &mut TextEngine) {
        let current = self.current_country();
        let wanted: Vec<(Country, RowKey)> = self
            .filtered()
            .into_iter()
            .map(|country| {
                let is_current = current.as_deref() == Some(country.code.as_str());
                let key = RowKey {
                    code: country.code.clone(),
                    sub: country_sub(&country, self.vpn(), is_current),
                    current: is_current,
                    favorite: self.vpn().favorites.contains(&country.code),
                    usable: country.online > 0,
                };
                (country, key)
            })
            .collect();
        let same = self.rows.len() == wanted.len()
            && self
                .rows
                .iter()
                .zip(&wanted)
                .all(|(row, (_, key))| row.key == *key);
        if same {
            return;
        }
        for row in self.rows.drain(..) {
            input.remove(row.area);
            input.remove(row.star.area);
            tree.remove(row.root);
        }
        for (country, key) in wanted {
            let root = rect(tree, self.list.content, Color::TRANSPARENT, 10.0);
            let hit = tree.add(root, Node::new(0.0, 0.0, 0.0, ROW));
            let area = input.add(MouseArea::new(hit));
            let badge = rect(
                tree,
                root,
                if key.current {
                    theme::green()
                } else {
                    theme::elevated()
                },
                7.0,
            );
            let code = label(
                tree,
                engine,
                badge,
                &country.code,
                11.0,
                if key.current {
                    theme::bg()
                } else {
                    theme::text()
                },
            );
            let name = label(
                tree,
                engine,
                root,
                &country.name,
                12.0,
                if key.usable {
                    theme::text()
                } else {
                    theme::muted()
                },
            );
            let sub = label(tree, engine, root, &key.sub, 10.0, theme::muted());
            let mut star = chip(
                tree,
                input,
                engine,
                root,
                if key.favorite { "󰓎" } else { "󰓒" },
                if key.favorite {
                    theme::yellow()
                } else {
                    theme::muted()
                },
                true,
                None,
            );
            star.shortcut = if key.favorite {
                "remove from favorites"
            } else {
                "add to favorites"
            }
            .into();
            self.rows.push(Row {
                key,
                root,
                hit,
                area,
                badge,
                code,
                name,
                sub,
                star,
            });
        }
    }

    pub fn sync(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        data: &PanelData,
        now: Millis,
    ) {
        self.data = data.clone();
        let vpn = self.data.vpn.clone();
        let fastest = vpn.fastest().cloned();
        let busy = vpn.state.busy();
        let connected = vpn.state == VpnState::On;
        let (power_text, power_ink) = match vpn.state {
            VpnState::On => ("disconnect", theme::red()),
            VpnState::Off => ("connect", theme::green()),
            VpnState::Connecting => ("connecting…", theme::yellow()),
            VpnState::Disconnecting => ("disconnecting…", theme::yellow()),
        };
        self.power.set_label(engine, power_text, 12.0);
        self.power.set_ink(now, power_ink);
        self.power.selected = connected;
        self.power.shortcut = match (&vpn.state, &fastest) {
            (VpnState::Off, Some(country)) => format!(
                "connect to the fastest country: {} · right click in the bar does the same",
                country.name
            ),
            (VpnState::On, _) => "disconnect from Proton VPN".into(),
            _ => String::new(),
        };
        self.refresh.set_label(
            engine,
            if vpn.refreshing || vpn.loading {
                "updating…"
            } else {
                "󰑐"
            },
            12.0,
        );
        let status = match vpn.state {
            VpnState::Connecting => format!("connecting to {}…", vpn.target),
            VpnState::Disconnecting => "disconnecting…".to_string(),
            VpnState::On => {
                let name = vpn
                    .connection
                    .as_ref()
                    .map_or("unknown server", |c| c.server.as_str())
                    .to_string();
                format!("protected · {name}")
            }
            VpnState::Off => match &fastest {
                Some(country) => format!("not connected · fastest is {}", country.name),
                None if vpn.loading => "not connected · reading the server list…".into(),
                None => "not connected".into(),
            },
        };
        self.status.set_text(engine, &status);
        self.status.set_color(match vpn.state {
            VpnState::On => theme::green(),
            VpnState::Off => theme::muted(),
            _ => theme::yellow(),
        });
        if connected {
            self.card_icon.set_text(engine, "󰕥");
            self.card_icon.set_color(theme::green());
            let (place, load, tags, country_code) = match &vpn.server {
                Some(server) => {
                    let mut place = vpn::country_name(&server.country);
                    if !server.city.is_empty() {
                        place = format!("{place} · {}", server.city);
                    }
                    (
                        place,
                        Some(server.load),
                        server.tags().join(" · "),
                        server.entry_country.clone(),
                    )
                }
                None => (
                    vpn.country()
                        .map_or("connected".into(), |code| vpn::country_name(&code)),
                    None,
                    String::new(),
                    String::new(),
                ),
            };
            self.place.set_text(engine, &place);
            match load {
                Some(load) => {
                    self.load.set_text(engine, &format!("load {load}%"));
                    self.load.set_color(load_color(load));
                }
                None => self.load.set_text(engine, ""),
            }
            let mut server_line = Vec::new();
            if let Some(connection) = &vpn.connection {
                server_line.push(connection.server.clone());
                server_line.push(vpn::protocol_label(&connection.protocol));
            }
            if let Some(server) = &vpn.server
                && server.entry_country != server.country
                && !country_code.is_empty()
            {
                server_line.push(format!("via {}", vpn::country_name(&country_code)));
            }
            if !tags.is_empty() {
                server_line.push(tags);
            }
            self.server.set_text(engine, &server_line.join(" · "));
            self.uptime.set_text(
                engine,
                &if vpn.uptime > 0 {
                    format!("󰔛 {}", duration(vpn.uptime))
                } else {
                    String::new()
                },
            );
            let ip = if vpn.ip.is_empty() {
                "server IP unknown".to_string()
            } else {
                format!("server {}", vpn.ip)
            };
            let domain = vpn
                .connection
                .as_ref()
                .map(|c| c.domain.clone())
                .filter(|d| !d.is_empty())
                .map_or(String::new(), |d| format!("  ·  {d}"));
            self.ip.set_text(engine, &format!("{ip}{domain}"));
            self.down
                .set_text(engine, &format!("↓ {}", speed(vpn.down.last().copied())));
            self.up
                .set_text(engine, &format!("↑ {}", speed(vpn.up.last().copied())));
            self.total.set_text(
                engine,
                &format!("session ↓ {}  ↑ {}", bytes(vpn.rx), bytes(vpn.tx)),
            );
        } else {
            self.card_icon
                .set_text(engine, if busy { "󰞀" } else { "󰒙" });
            self.card_icon.set_color(if busy {
                theme::yellow()
            } else {
                theme::muted()
            });
            self.place.set_text(
                engine,
                if busy {
                    "working on it…"
                } else {
                    "your traffic is not protected"
                },
            );
            self.load.set_text(engine, "");
            let home = vpn.home.as_deref().map(vpn::country_name);
            let skipped: Vec<String> = vpn
                .exclude
                .iter()
                .filter(|code| Some(code.as_str()) != vpn.home.as_deref())
                .map(|code| vpn::country_name(code))
                .collect();
            let mut line = match &home {
                Some(home) => format!("your location: {home}"),
                None => "your location is unknown".into(),
            };
            if !skipped.is_empty() {
                line.push_str(&format!(" · also skipped: {}", skipped.join(", ")));
            }
            self.server.set_text(engine, &line);
            self.uptime.set_text(engine, "");
            let fastest_line = match &fastest {
                Some(country) => format!(
                    "fastest outside it: {} · {} servers · load {}%",
                    country.name, country.online, country.load
                ),
                None => String::new(),
            };
            self.ip.set_text(engine, &fastest_line);
            let catalog_line = vpn.catalog.as_ref().map_or(String::new(), |catalog| {
                let age = vpn::now().saturating_sub(catalog.updated);
                format!(
                    "{} countries · {} servers · list updated {} ago",
                    catalog.countries.len(),
                    catalog.servers,
                    duration(age)
                )
            });
            self.total.set_text(engine, &catalog_line);
            self.down.set_text(engine, "");
            self.up.set_text(engine, "");
        }
        for (item, _) in &mut self.quick {
            if let Some(area) = input.get_mut(item.area) {
                area.enabled = !busy;
            }
        }
        for item in &mut self.settings {
            let value = vpn.settings.get(item.key).cloned().unwrap_or_default();
            let pending = vpn.pending_setting == item.key;
            let text = if pending {
                format!("{}: …", setting_title(item.key))
            } else {
                format!("{}: {}", setting_title(item.key), setting_value(&value))
            };
            item.chip.set_label(engine, &text, 11.0);
            let on = !value.is_empty() && value != "off";
            item.chip
                .set_ink(now, if on { theme::green() } else { theme::muted() });
            item.chip.shortcut = setting_help(item.key, &value);
            if let Some(area) = input.get_mut(item.chip.area) {
                area.enabled = !value.is_empty() && vpn.pending_setting.is_empty();
            }
        }
        self.all.selected = self.section == Section::All;
        self.favorites.selected = self.section == Section::Favorites;
        self.recent.selected = self.section == Section::Recent;
        self.sort_chip.set_label(
            engine,
            match self.sort {
                Sort::Name => "󰖽 a–z",
                Sort::Speed => "󰓅 fastest",
            },
            12.0,
        );
        let empty = if vpn.catalog.is_none() {
            if vpn.loading {
                "reading the server list…"
            } else {
                "no server list · sign in with protonvpn signin"
            }
        } else {
            match self.section {
                Section::All => "no country matches",
                Section::Favorites => "no favorites yet · tap a star",
                Section::Recent => "no recent connections",
            }
        };
        self.empty.set_text(engine, empty);
        self.message.set_text(engine, &vpn.message);
        self.rebuild(tree, input, engine);
        for row in &self.rows {
            if let Some(area) = input.get_mut(row.area) {
                area.enabled = !busy && row.key.usable;
            }
        }
    }

    pub fn layout(&mut self, tree: &mut Tree, engine: &mut TextEngine, width: f32, height: f32) {
        tree.set_size(self.root, width, height);
        let vpn = self.vpn().clone();
        let connected = vpn.state == VpnState::On;
        let message_visible = !vpn.message.is_empty();
        let header_h = self
            .title
            .implicit_height()
            .ceil()
            .max(self.power.implicit_height());
        let message_h = if message_visible {
            self.message.wrap(engine, width, 3)
        } else {
            0.0
        };
        let card_h = if connected { 172.0 } else { 104.0 };
        let mut cells = vec![
            Cell::layout(header_h),
            Cell::implicit(self.status.implicit_height()),
            Cell::preferred(card_h),
            Cell::layout(30.0),
            Cell::layout(30.0),
            Cell::layout(30.0),
            Cell::layout(38.0),
            Cell::fill(0.0),
        ];
        if message_visible {
            cells.push(Cell::implicit(message_h));
        }
        let slots = linear(&cells, 10.0, height);
        let mut next = slots.iter().copied();
        let header = next.next().unwrap_or_default();
        let columns = linear(
            &[
                Cell::fill(self.title.implicit_width().ceil()),
                Cell::implicit(self.refresh.implicit_width()),
                Cell::implicit(self.power.implicit_width()),
            ],
            5.0,
            width,
        );
        place_label(
            tree,
            &mut self.title,
            columns[0].pos,
            header,
            columns[0].size,
        );
        place_chip(tree, &mut self.refresh, columns[1].pos, header);
        place_chip(tree, &mut self.power, columns[2].pos, header);
        let status = next.next().unwrap_or_default();
        place_label(tree, &mut self.status, 0.0, status, width);
        let slot = next.next().unwrap_or_default();
        let (y, h) = fit(slot, card_h);
        tree.set_pos(self.card, 0.0, y);
        tree.set_size(self.card, width, h);
        self.layout_card(tree, width, h, connected);

        let quick = next.next().unwrap_or_default();
        let mut quick_cells: Vec<Cell> = Vec::new();
        if connected {
            quick_cells.push(Cell::implicit(self.reconnect.implicit_width()));
        }
        quick_cells.extend(
            self.quick
                .iter()
                .map(|(item, _)| Cell::implicit(item.implicit_width())),
        );
        quick_cells.push(Cell::fill(0.0));
        let columns = linear(&quick_cells, 5.0, width);
        let mut column = columns.iter();
        tree.set_visible(self.reconnect.root, connected);
        if connected && let Some(cell) = column.next() {
            place_chip(tree, &mut self.reconnect, cell.pos, quick);
        }
        for ((item, _), cell) in self.quick.iter_mut().zip(column) {
            place_chip(tree, item, cell.pos, quick);
        }

        let settings = next.next().unwrap_or_default();
        let mut cells: Vec<Cell> = self
            .settings
            .iter()
            .map(|item| Cell::implicit(item.chip.implicit_width()))
            .collect();
        cells.push(Cell::fill(0.0));
        let columns = linear(&cells, 5.0, width);
        for (item, cell) in self.settings.iter_mut().zip(&columns) {
            place_chip(tree, &mut item.chip, cell.pos, settings);
        }

        let tabs = next.next().unwrap_or_default();
        let columns = linear(
            &[
                Cell::implicit(self.all.implicit_width()),
                Cell::implicit(self.favorites.implicit_width()),
                Cell::implicit(self.recent.implicit_width()),
                Cell::fill(0.0),
                Cell::implicit(self.sort_chip.implicit_width()),
            ],
            5.0,
            width,
        );
        place_chip(tree, &mut self.all, columns[0].pos, tabs);
        place_chip(tree, &mut self.favorites, columns[1].pos, tabs);
        place_chip(tree, &mut self.recent, columns[2].pos, tabs);
        place_chip(tree, &mut self.sort_chip, columns[4].pos, tabs);

        let search = next.next().unwrap_or_default();
        tree.set_pos(self.search_frame, 0.0, search.pos);
        tree.set_size(self.search_frame, width, 38.0);
        set_border(
            tree,
            self.search_frame,
            if self.search.has_focus() {
                theme::green()
            } else {
                theme::line()
            },
            1.0,
        );
        self.search
            .place(tree, engine, 10.0, 10.0, width - 20.0, 18.0);
        tree.set_visible(self.placeholder.node, self.search.text().is_empty());
        let ph = self.placeholder.implicit_height().ceil();
        self.placeholder.place(tree, 10.0, 10.0, width - 20.0, ph);

        let list = next.next().unwrap_or_default();
        let mut y = 0.0;
        for (index, row) in self.rows.iter_mut().enumerate() {
            if index > 0 {
                y += 4.0;
            }
            tree.set_pos(row.root, 0.0, y);
            tree.set_size(row.root, width, ROW);
            tree.set_size(row.hit, width - 44.0, ROW);
            Self::layout_row(tree, row, width);
            y += ROW;
        }
        self.list.place(tree, 0.0, list.pos, width, list.size, y);
        tree.set_visible(self.empty.node, self.rows.is_empty());
        let ew = self.empty.implicit_width();
        let eh = self.empty.implicit_height().ceil();
        self.empty
            .place(tree, center(width, ew), center(list.size, eh), ew, eh);
        tree.set_visible(self.message.node, message_visible);
        if message_visible {
            let slot = next.next().unwrap_or_default();
            self.message.place(tree, 0.0, slot.pos, width, message_h);
        }
    }

    fn layout_card(&mut self, tree: &mut Tree, width: f32, height: f32, connected: bool) {
        let inner = width - 24.0;
        let line = |label: &Label| label.implicit_height().ceil();
        let row1_h = line(&self.card_icon)
            .max(line(&self.place))
            .max(line(&self.load));
        let row2_h = line(&self.server).max(line(&self.uptime));
        let row3_h = line(&self.ip).max(if connected {
            self.copy.implicit_height()
        } else {
            0.0
        });
        let row4_h = line(&self.down).max(line(&self.total));
        let mut cells = vec![
            Cell::layout(row1_h),
            Cell::layout(row2_h),
            Cell::layout(row3_h),
        ];
        if connected {
            cells.push(Cell::fill(40.0).margins(2.0, 0.0));
        }
        cells.push(Cell::layout(row4_h));
        let rows = linear(&cells, 5.0, height - 24.0);
        let shift = |slot: Slot| Slot {
            pos: slot.pos + 12.0,
            size: slot.size,
        };
        let row = shift(rows[0]);
        let columns = linear(
            &[
                Cell::implicit(self.card_icon.implicit_width()),
                Cell::fill(self.place.implicit_width().ceil()),
                Cell::implicit(self.load.implicit_width()),
            ],
            8.0,
            inner,
        );
        place_label(
            tree,
            &mut self.card_icon,
            12.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.place,
            12.0 + columns[1].pos,
            row,
            columns[1].size,
        );
        place_label(
            tree,
            &mut self.load,
            12.0 + columns[2].pos,
            row,
            columns[2].size,
        );
        let row = shift(rows[1]);
        let columns = linear(
            &[
                Cell::fill(self.server.implicit_width().ceil()),
                Cell::implicit(self.uptime.implicit_width()),
            ],
            5.0,
            inner,
        );
        place_label(
            tree,
            &mut self.server,
            12.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        place_label(
            tree,
            &mut self.uptime,
            12.0 + columns[1].pos,
            row,
            columns[1].size,
        );
        let row = shift(rows[2]);
        let mut cells = vec![Cell::fill(self.ip.implicit_width().ceil())];
        if connected {
            cells.push(Cell::implicit(self.copy.implicit_width()));
        }
        let columns = linear(&cells, 5.0, inner);
        place_label(
            tree,
            &mut self.ip,
            12.0 + columns[0].pos,
            row,
            columns[0].size,
        );
        tree.set_visible(self.copy.root, connected && !self.vpn().ip.is_empty());
        if let Some(cell) = columns.get(1) {
            place_chip(tree, &mut self.copy, 12.0 + cell.pos, row);
        }
        tree.set_visible(self.spark.root, connected);
        let mut index = 3;
        if connected {
            let spark = shift(rows[3]);
            let vpn = self.data.vpn.clone();
            self.spark
                .place(tree, 12.0, spark.pos, inner, spark.size, &vpn.down, &vpn.up);
            index = 4;
        }
        let row = shift(rows[index]);
        let mut cells = Vec::new();
        if connected {
            cells.push(Cell::implicit(self.down.implicit_width()));
            cells.push(Cell::implicit(self.up.implicit_width()));
        }
        cells.push(Cell::fill(0.0));
        cells.push(Cell::implicit(self.total.implicit_width()));
        let columns = linear(&cells, 12.0, inner);
        tree.set_visible(self.down.node, connected);
        tree.set_visible(self.up.node, connected);
        if connected {
            place_label(
                tree,
                &mut self.down,
                12.0 + columns[0].pos,
                row,
                columns[0].size,
            );
            place_label(
                tree,
                &mut self.up,
                12.0 + columns[1].pos,
                row,
                columns[1].size,
            );
        }
        let last = columns.len() - 1;
        let total_x = if connected { columns[last].pos } else { 0.0 };
        let total_w = if connected { columns[last].size } else { inner };
        place_label(tree, &mut self.total, 12.0 + total_x, row, total_w);
    }

    fn layout_row(tree: &mut Tree, row: &mut Row, width: f32) {
        let inner = Slot {
            pos: 8.0,
            size: ROW - 16.0,
        };
        let text_h = row.name.implicit_height().ceil() + 3.0 + row.sub.implicit_height().ceil();
        let columns = linear(
            &[
                Cell::implicit(34.0),
                Cell::fill(
                    row.name
                        .implicit_width()
                        .ceil()
                        .max(row.sub.implicit_width().ceil()),
                ),
                Cell::implicit(row.star.implicit_width()),
            ],
            10.0,
            width - 20.0,
        );
        let (by, bh) = fit(inner, 26.0);
        tree.set_pos(row.badge, 10.0 + columns[0].pos, by);
        tree.set_size(row.badge, 34.0, bh);
        let cw = row.code.implicit_width().ceil();
        let ch = row.code.implicit_height().ceil();
        row.code
            .place(tree, center(34.0, cw), center(bh, ch), cw, ch);
        let (ty, _) = fit(inner, text_h);
        let nh = row.name.implicit_height().ceil();
        row.name
            .place(tree, 10.0 + columns[1].pos, ty, columns[1].size, nh);
        let sh = row.sub.implicit_height().ceil();
        row.sub.place(
            tree,
            10.0 + columns[1].pos,
            ty + nh + 3.0,
            columns[1].size,
            sh,
        );
        place_chip(tree, &mut row.star, 10.0 + columns[2].pos, inner);
    }

    pub fn tick(&mut self, tree: &mut Tree, input: &Input, now: Millis) -> bool {
        let mut running = false;
        let mut chips: Vec<&mut Chip> = vec![
            &mut self.refresh,
            &mut self.power,
            &mut self.copy,
            &mut self.reconnect,
            &mut self.all,
            &mut self.favorites,
            &mut self.recent,
            &mut self.sort_chip,
        ];
        chips.extend(self.quick.iter_mut().map(|(chip, _)| chip));
        chips.extend(self.settings.iter_mut().map(|item| &mut item.chip));
        for row in &mut self.rows {
            chips.push(&mut row.star);
        }
        for chip in chips {
            chip.sync(now, input);
            running |= chip.tick(tree, now);
        }
        for row in &self.rows {
            let color = if row.key.current {
                theme::elevated()
            } else if input.contains_mouse(row.area) && row.key.usable {
                theme::surface()
            } else {
                Color::TRANSPARENT
            };
            tree.set_fill(row.root, color);
            tree.set_opacity(row.root, if row.key.usable { 1.0 } else { 0.55 });
            for label in [&row.code, &row.name, &row.sub] {
                label.sync(tree);
            }
        }
        running |= self.list.tick(tree, input, now);
        self.search.tick(tree, now);
        for label in [
            &self.title,
            &self.status,
            &self.card_icon,
            &self.place,
            &self.load,
            &self.server,
            &self.uptime,
            &self.ip,
            &self.down,
            &self.up,
            &self.total,
            &self.placeholder,
            &self.empty,
            &self.message,
        ] {
            label.sync(tree);
        }
        running
    }

    pub fn next_deadline(&self, now: Millis) -> Option<Millis> {
        self.search.next_blink(now)
    }

    fn connect_row(&self, code: &str, actions: &mut Vec<PanelAction>) {
        if self.current_country().as_deref() == Some(code) {
            actions.push(PanelAction::Vpn(VpnAction::Disconnect));
        } else {
            actions.push(PanelAction::Vpn(VpnAction::Connect(Target::Country(
                code.to_string(),
            ))));
        }
    }

    pub fn handle(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        events: &[InputEvent],
        pointer: (f32, f32),
        now: Millis,
    ) -> Vec<PanelAction> {
        let mut actions = Vec::new();
        let vpn = self.vpn().clone();
        for event in events {
            match *event {
                InputEvent::Press(area, MouseButton::Left) if area == self.search.area => {
                    self.search.set_focus(true, now);
                    self.search.press(tree, engine, pointer.0, pointer.1, now);
                }
                InputEvent::Wheel(area, delta) if area == self.list.area => {
                    self.list.wheel(delta, now);
                }
                InputEvent::Click(area, MouseButton::Left) => {
                    if area == self.power.area {
                        match vpn.state {
                            VpnState::On => actions.push(PanelAction::Vpn(VpnAction::Disconnect)),
                            VpnState::Off => actions.push(PanelAction::Vpn(VpnAction::Fastest)),
                            _ => {}
                        }
                    } else if area == self.refresh.area {
                        actions.push(PanelAction::Vpn(VpnAction::Refresh));
                    } else if area == self.copy.area {
                        if !vpn.ip.is_empty() {
                            actions.push(PanelAction::Vpn(VpnAction::CopyIp(vpn.ip.clone())));
                        }
                    } else if area == self.reconnect.area {
                        actions.push(PanelAction::Vpn(VpnAction::Reconnect));
                    } else if area == self.all.area {
                        self.section = Section::All;
                        self.list.jump(0.0);
                    } else if area == self.favorites.area {
                        self.section = Section::Favorites;
                        self.list.jump(0.0);
                    } else if area == self.recent.area {
                        self.section = Section::Recent;
                        self.list.jump(0.0);
                    } else if area == self.sort_chip.area {
                        self.sort = match self.sort {
                            Sort::Name => Sort::Speed,
                            Sort::Speed => Sort::Name,
                        };
                        self.list.jump(0.0);
                    } else if let Some((_, target)) =
                        self.quick.iter().find(|(chip, _)| chip.area == area)
                    {
                        actions.push(PanelAction::Vpn(VpnAction::Connect(target.clone())));
                    } else if let Some(item) =
                        self.settings.iter().find(|item| item.chip.area == area)
                    {
                        actions.push(PanelAction::Vpn(VpnAction::Setting(item.key.to_string())));
                    } else if let Some(row) = self.rows.iter().find(|row| row.star.area == area) {
                        actions.push(PanelAction::Vpn(VpnAction::Favorite(row.key.code.clone())));
                    } else if let Some(row) = self.rows.iter().find(|row| row.area == area) {
                        let code = row.key.code.clone();
                        self.connect_row(&code, &mut actions);
                    }
                }
                _ => {}
            }
        }
        let data = self.data.clone();
        self.sync(tree, input, engine, &data, now);
        actions
    }

    pub fn motion(
        &mut self,
        tree: &mut Tree,
        engine: &mut TextEngine,
        input: &Input,
        x: f32,
        y: f32,
    ) {
        self.search.drag(tree, engine, input, x, y);
    }

    pub fn key(
        &mut self,
        tree: &mut Tree,
        input: &mut Input,
        engine: &mut TextEngine,
        event: &KeyEvent,
        now: Millis,
    ) -> (bool, Vec<PanelAction>) {
        let mut actions = Vec::new();
        let before = self.search.text().to_string();
        match self.search.key(tree, engine, event, now) {
            KeyResult::Accepted => {
                if let Some(row) = self.rows.iter().find(|row| row.key.usable) {
                    let code = row.key.code.clone();
                    self.connect_row(&code, &mut actions);
                }
            }
            KeyResult::Escape => {
                if before.is_empty() {
                    return (false, actions);
                }
                self.search.set_text(tree, engine, "");
            }
            KeyResult::Handled => {}
            KeyResult::Ignored => return (false, actions),
        }
        if self.search.text() != before {
            self.list.jump(0.0);
            let data = self.data.clone();
            self.sync(tree, input, engine, &data, now);
        }
        (true, actions)
    }

    pub fn tooltip(&self, input: &Input) -> Option<TooltipSpot> {
        let mut chips: Vec<&Chip> = vec![
            &self.refresh,
            &self.power,
            &self.copy,
            &self.reconnect,
            &self.sort_chip,
        ];
        chips.extend(self.quick.iter().map(|(chip, _)| chip));
        chips.extend(self.settings.iter().map(|item| &item.chip));
        chips.extend(self.rows.iter().map(|row| &row.star));
        chips
            .into_iter()
            .find(|chip| !chip.shortcut.is_empty() && chip.hovered(input))
            .map(|chip| TooltipSpot {
                key: chip.area,
                text: chip.shortcut.clone(),
                node: chip.root,
                width: chip.width(),
                height: chip.height(),
            })
    }
}
