use std::path::PathBuf;

use platform::Damage;
use ui::{Canvas, TextEngine};

use crate::{bar::Bar, model::BarModel, overlay::Overlay, panels::PanelData, theme::font_families};

const BACKDROP: [u8; 3] = [0x5a, 0x55, 0x4c];

fn target() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("CHUHSHELL_PREVIEW_DIR")?);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn save(dir: &std::path::Path, name: &str, data: &[u8], width: u32, height: u32) {
    let mut out = Vec::with_capacity(data.len());
    for pixel in data.as_chunks::<4>().0 {
        let alpha = pixel[3] as u32;
        for channel in 0..3 {
            let value = pixel[channel] as u32 + BACKDROP[channel] as u32 * (255 - alpha) / 255;
            out.push(value.min(255) as u8);
        }
        out.push(255);
    }
    if let Some(image) = image::RgbaImage::from_raw(width, height, out) {
        let _ = image.save(dir.join(format!("{name}.png")));
    }
}

fn bar_model() -> BarModel {
    BarModel {
        title: Some("chuhshell — visual pass".into()),
        title_key: "kitty".into(),
        workspaces: vec![1, 2, 3, 4, 5],
        occupied: vec![1, 3],
        active_workspace: 1,
        cpu: Some(37),
        ram_used: 5.4,
        ram_total: 15.5,
        keyboard_label: "EN".into(),
        wifi_strength: 80.0,
        bluetooth_powered: true,
        volume: 48,
        battery_level: 76,
        battery_plugged: false,
        unread_notifications: 2,
        media: Some(("Midnight City · M83".into(), true)),
        ..BarModel::placeholder()
    }
}

fn panel_data() -> PanelData {
    PanelData {
        time: "22:49".into(),
        date: "Saturday, 14 November".into(),
        network_connected: true,
        network_name: "home-5g".into(),
        bluetooth_powered: true,
        bluetooth_name: "train".into(),
        volume: 48,
        brightness: 80,
        keep_awake: true,
        media: Some(services::media::Track {
            player: "org.mpris.MediaPlayer2.mpv".into(),
            title: "Midnight City".into(),
            artist: "M83".into(),
            art: String::new(),
            playing: true,
            can_next: true,
            can_previous: false,
        }),
        resources: services::system::Resources {
            cpu: Some(37),
            ram_used: 5.4,
            ram_total: 15.5,
        },
        cpu_history: (0..60)
            .map(|i| 20.0 + (i as f64 * 0.7).sin() * 15.0)
            .collect(),
        ram_history: (0..60)
            .map(|i| 35.0 + (i as f64 * 0.2).cos() * 4.0)
            .collect(),
        theme_available: true,
        theme_scheme: "tonal-spot",
        theme_palette: "gruvbox".into(),
        cursors: std::sync::Arc::new(crate::cursorctl::CursorData {
            items: [
                "Bibata-Modern-Classic",
                "Capitaine Cursors",
                "Hu Tao",
                "Phinger",
            ]
            .iter()
            .map(|name| crate::cursorctl::CursorItem {
                theme: services::cursors::CursorTheme {
                    name: name.to_string(),
                    path: String::new(),
                    revision: String::new(),
                    animated: *name == "Hu Tao",
                },
                picture: String::new(),
            })
            .collect(),
            current: "Capitaine Cursors".into(),
            size: 24,
            colors: true,
            animation: true,
            ..Default::default()
        }),
        ..PanelData::default()
    }
}

#[test]
#[ignore]
fn frame_cost() {
    let mut engine = TextEngine::new(&font_families());
    let data = panel_data();
    let mut total = std::time::Duration::ZERO;
    for panel in [
        "control",
        "calendar",
        "battery",
        "audio",
        "system",
        "power",
        "notifications",
    ] {
        for round in 0..3 {
            if let Some((spent, frames)) =
                Overlay::frame_cost(&mut engine, panel, &data, (1920.0, 1080.0))
            {
                if round > 0 {
                    total += spent;
                }
                println!("{panel} round {round}: {frames} frames in {spent:?}");
            }
        }
    }
    println!("total {total:?}");
}

#[test]
#[ignore]
fn dump() {
    let Some(dir) = target() else {
        return;
    };
    crate::theme::apply_config(&config::current());
    let mut engine = TextEngine::new(&font_families());
    let scale = crate::theme::scale();
    let (width, height) = (1896u32, (28.0 * scale).ceil() as u32);
    let mut bar = Bar::new(&mut engine, width, height, &bar_model(), 0.0);
    let mut now = 0.0;
    while now < 2000.0 {
        bar.tick(now);
        now += 16.0;
    }
    if let Some(mut canvas) = Canvas::new(width, height) {
        bar.tree.collect_damage();
        canvas.render(
            &mut bar.tree,
            &mut engine,
            &Damage::full(width as i32, height as i32),
        );
        save(&dir, "bar", canvas.data(), width, height);
    }
    let panels = std::env::var("CHUHSHELL_PREVIEW_PANELS")
        .unwrap_or_else(|_| "control,calendar,battery,audio,system,power,notifications".into());
    let until = std::env::var("CHUHSHELL_PREVIEW_UNTIL")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3000.0);
    let data = panel_data();
    for panel in panels.split(',') {
        let mut overlay = Overlay::new();
        if let Some((pixels, w, h)) =
            overlay.preview(&mut engine, panel, &data, (1920.0, 1080.0), until)
        {
            save(&dir, panel, &pixels, w, h);
        }
    }
}
