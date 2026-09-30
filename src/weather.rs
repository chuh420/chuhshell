use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    #[default]
    System,
    Celsius,
    Fahrenheit,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Location {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Default for Location {
    fn default() -> Self {
        Self {
            name: "Aktobe, Kazakhstan".into(),
            latitude: 50.2839,
            longitude: 57.1670,
        }
    }
}

fn json(url: &str) -> Result<Value, String> {
    let body = crate::process::run_with_timeout(
        "curl",
        &[
            "--fail",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "5",
            "--max-time",
            "12",
            url,
        ],
        std::time::Duration::from_secs(14),
    )?;
    serde_json::from_str(&body).map_err(|_| "Invalid weather response".into())
}

fn search(query: &str) -> Result<Vec<Location>, String> {
    let query = glib::uri_escape_string(query, None::<&str>, false);
    let data = json(&format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={query}&count=6&language=en&format=json"
    ))?;
    Ok(data["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|city| {
            Some(Location {
                name: format!(
                    "{}, {}{}",
                    city["name"].as_str()?,
                    city["admin1"]
                        .as_str()
                        .map(|s| format!("{s}, "))
                        .unwrap_or_default(),
                    city["country"].as_str().unwrap_or("")
                ),
                latitude: city["latitude"].as_f64()?,
                longitude: city["longitude"].as_f64()?,
            })
        })
        .collect())
}

fn system_city() -> String {
    let zone = glib::TimeZone::local().identifier().to_string();
    let city = zone
        .rsplit('/')
        .next()
        .unwrap_or("Aktobe")
        .replace('_', " ");
    match city.as_str() {
        "Aqtobe" => "Aktobe".into(),
        "UTC" | "Etc" | "GMT" => String::new(),
        _ => city,
    }
}

fn fahrenheit() -> bool {
    let locale = ["LC_ALL", "LC_MEASUREMENT", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|s| !s.is_empty()))
        .unwrap_or_default();
    locale.contains("_US") || locale.contains("_LR") || locale.contains("_MM")
}

fn conditions(code: i64) -> &'static str {
    match code {
        0 => "Clear sky",
        1 | 2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51..=57 => "Drizzle",
        61..=67 => "Rain",
        71..=77 => "Snow",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95..=99 => "Thunderstorm",
        _ => "Unknown conditions",
    }
}

fn forecast(location: &Location, imperial: bool) -> Result<Report, String> {
    if !location.latitude.is_finite()
        || !location.longitude.is_finite()
        || location.latitude.abs() > 90.0
        || location.longitude.abs() > 180.0
    {
        return Err("Invalid location coordinates".into());
    }
    let unit = if imperial { "fahrenheit" } else { "celsius" };
    let data = json(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=auto&forecast_days=5&temperature_unit={unit}",
        location.latitude, location.longitude
    ))?;
    format_forecast(&data)
}

struct Report {
    temperature: String,
    conditions: &'static str,
    detail: String,
    days: Vec<(String, String, String)>,
    updated: String,
}

fn format_forecast(data: &Value) -> Result<Report, String> {
    let current = &data["current"];
    let temperature = current["temperature_2m"]
        .as_f64()
        .ok_or("Weather data unavailable")?;
    let unit = data["current_units"]["temperature_2m"]
        .as_str()
        .unwrap_or("°C");
    let number = |name: &str| {
        current[name]
            .as_f64()
            .map(|n| format!("{n:.0}"))
            .unwrap_or_else(|| "—".into())
    };
    let detail = format!(
        "Feels like {}{unit}  ·  Humidity {}%\nWind {} km/h",
        number("apparent_temperature"),
        number("relative_humidity_2m"),
        number("wind_speed_10m")
    );
    let daily = &data["daily"];
    let days = daily["time"]
        .as_array()
        .into_iter()
        .flatten()
        .take(5)
        .enumerate()
        .map(|(i, date)| {
            let number = |name: &str| {
                daily[name][i]
                    .as_f64()
                    .map(|n| format!("{n:.0}"))
                    .unwrap_or_else(|| "—".into())
            };
            let date = date.as_str().unwrap_or("");
            let day = glib::DateTime::from_iso8601(&format!("{date}T12:00:00Z"), None)
                .and_then(|day| day.format("%a, %e %b"))
                .map(|day| day.to_string())
                .unwrap_or_else(|_| date.into());
            (
                day,
                format!(
                    "{} / {}{unit}",
                    number("temperature_2m_min"),
                    number("temperature_2m_max")
                ),
                format!(
                    "{} · {}% precipitation",
                    conditions(daily["weather_code"][i].as_i64().unwrap_or(-1)),
                    number("precipitation_probability_max")
                ),
            )
        })
        .collect();
    Ok(Report {
        temperature: format!("{temperature:.0}{unit}"),
        conditions: conditions(current["weather_code"].as_i64().unwrap_or(-1)),
        detail,
        days,
        updated: format!(
            "Updated {} · {}",
            current["time"].as_str().unwrap_or(""),
            data["timezone"].as_str().unwrap_or("local time")
        ),
    })
}

fn render_report(container: &gtk::Box, report: Report) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let current = gtk::Box::new(gtk::Orientation::Vertical, 6);
    current.add_css_class("weather-current");
    current.append(&crate::info::label(
        &report.temperature,
        "weather-temperature",
    ));
    current.append(&crate::info::label(report.conditions, "menu-title"));
    current.append(&crate::info::label(&report.detail, "menu-hint"));
    container.append(&current);
    for (day, temperatures, conditions) in report.days {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.add_css_class("weather-day");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let date = crate::info::label(&day, "menu-title");
        date.set_hexpand(true);
        line.append(&date);
        line.append(&crate::info::label(&temperatures, "weather-range"));
        row.append(&line);
        row.append(&crate::info::label(&conditions, "menu-hint"));
        container.append(&row);
    }
    container.append(&crate::info::label(&report.updated, "menu-hint"));
}

pub fn view() -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let settings = crate::config::read().unwrap_or_default();
    let location = Rc::new(std::cell::RefCell::new(
        settings.weather_location.clone().unwrap_or_default(),
    ));
    let system = Rc::new(Cell::new(settings.weather_system));
    let units = Rc::new(Cell::new(match settings.weather_units {
        Units::System => 0,
        Units::Celsius => 1,
        Units::Fahrenheit => 2,
    }));
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::builder()
        .placeholder_text("City name")
        .hexpand(true)
        .max_length(100)
        .build();
    entry.add_css_class("search");
    let find = crate::info::button("Search");
    controls.append(&entry);
    controls.append(&find);
    outer.append(&controls);
    let options = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let automatic = crate::info::button("Use system timezone");
    let refresh = crate::info::button("Refresh");
    options.append(&automatic);
    options.append(&refresh);
    let unit_selector = gtk::DropDown::from_strings(&["System units", "°C", "°F"]);
    unit_selector.set_selected(units.get());
    unit_selector.set_tooltip_text(Some("Temperature units"));
    options.append(&unit_selector);
    outer.append(&options);
    let heading = crate::info::label("", "menu-title");
    outer.append(&heading);
    let results = gtk::Box::new(gtk::Orientation::Vertical, 5);
    let results_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(250)
        .max_content_height(350)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&results)
        .build();
    results_scroll.set_visible(false);
    outer.append(&results_scroll);
    let status = crate::info::label("", "menu-hint");
    outer.append(&status);
    let report = gtk::Box::new(gtk::Orientation::Vertical, 8);
    report.add_css_class("weather-report");
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(250)
        .max_content_height(350)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&report)
        .build();
    outer.append(&scroll);
    let attribution = gtk::LinkButton::with_label(
        "https://open-meteo.com/",
        "Weather: Open-Meteo · Locations: GeoNames",
    );
    attribution.add_css_class("menu-hint");
    outer.append(&attribution);
    let busy = Rc::new(Cell::new(false));
    let update: Rc<dyn Fn()> = Rc::new({
        let location = location.clone();
        let units = units.clone();
        let system = system.clone();
        let busy = busy.clone();
        let results_scroll = results_scroll.downgrade();
        let report_scroll = scroll.downgrade();
        let heading = heading.downgrade();
        let report = report.downgrade();
        let status = status.downgrade();
        move || {
            if busy.replace(true) {
                return;
            }
            let (Some(heading), Some(report), Some(status)) =
                (heading.upgrade(), report.upgrade(), status.upgrade())
            else {
                busy.set(false);
                return;
            };
            if let Some(scroll) = results_scroll.upgrade() {
                scroll.set_visible(false);
            }
            if let Some(scroll) = report_scroll.upgrade() {
                scroll.set_visible(true);
            }
            while let Some(child) = report.first_child() {
                report.remove(&child);
            }
            heading.set_text(&location.borrow().name);
            status.set_text("Loading weather…");
            let mut city = location.borrow().clone();
            let auto = system.get();
            let system_query = system_city();
            let imperial = match units.get() {
                1 => false,
                2 => true,
                _ => fahrenheit(),
            };
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let result = (|| -> Result<(Location, Report), String> {
                    if auto {
                        if system_query.is_empty() {
                            return Err(
                                "System timezone has no city. Choose a city manually.".into()
                            );
                        }
                        city = search(&system_query)?
                            .into_iter()
                            .next()
                            .ok_or("Could not match the timezone city. Choose a city manually.")?;
                    }
                    let report = forecast(&city, imperial)?;
                    Ok((city, report))
                })();
                let _ = tx.send_blocking(result);
            });
            let location = location.clone();
            let busy = busy.clone();
            let heading = heading.downgrade();
            let report = report.downgrade();
            let status = status.downgrade();
            glib::MainContext::default().spawn_local(async move {
                let result = rx.recv().await;
                busy.set(false);
                let (Some(heading), Some(report), Some(status)) =
                    (heading.upgrade(), report.upgrade(), status.upgrade())
                else {
                    return;
                };
                match result {
                    Ok(Ok((city, text))) => {
                        heading.set_text(&city.name);
                        *location.borrow_mut() = city;
                        render_report(&report, text);
                        status.set_text(if auto {
                            "City inferred from system timezone"
                        } else {
                            "Selected city"
                        });
                    }
                    Ok(Err(error)) => status.set_text(&format!("Could not update: {error}")),
                    Err(_) => status.set_text("Weather worker stopped"),
                }
            });
        }
    });
    unit_selector.connect_selected_notify({
        let units = units.clone();
        let busy = busy.clone();
        let update = update.clone();
        let status = status.downgrade();
        move |selector| {
            let selected = selector.selected();
            if selected == units.get() {
                return;
            }
            if busy.get() {
                selector.set_selected(units.get());
                return;
            }
            let value = match selected {
                1 => Units::Celsius,
                2 => Units::Fahrenheit,
                _ => Units::System,
            };
            busy.set(true);
            selector.set_sensitive(false);
            let saved = crate::config::save_value_async(
                "weather_units",
                serde_json::to_value(value).unwrap(),
            );
            let selector = selector.downgrade();
            let units = units.clone();
            let busy = busy.clone();
            let update = update.clone();
            let status = status.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = saved.await;
                busy.set(false);
                if let Some(selector) = selector.upgrade() {
                    selector.set_sensitive(true);
                    match result {
                        Ok(()) => {
                            units.set(selected);
                            update();
                        }
                        Err(error) => {
                            selector.set_selected(units.get());
                            if let Some(status) = status.upgrade() {
                                status.set_text(&error);
                            }
                        }
                    }
                }
            });
        }
    });
    refresh.connect_clicked({
        let update = update.clone();
        move |_| update()
    });
    automatic.connect_clicked({
        let system = system.clone();
        let update = update.clone();
        let status = status.downgrade();
        let busy = busy.clone();
        move |_| {
            if busy.get() {
                return;
            }
            busy.set(true);
            let saved = crate::config::save_value_async("weather_system", true.into());
            let busy = busy.clone();
            let system = system.clone();
            let update = update.clone();
            let status = status.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = saved.await;
                busy.set(false);
                match result {
                    Ok(()) => {
                        system.set(true);
                        update();
                    }
                    Err(error) => {
                        if let Some(status) = status.upgrade() {
                            status.set_text(&error);
                        }
                    }
                }
            });
        }
    });
    find.connect_clicked({
        let results_scroll = results_scroll.downgrade();
        let report_scroll = scroll.downgrade();
        let entry = entry.downgrade();
        let results = results.downgrade();
        let status = status.downgrade();
        let busy = busy.clone();
        let location = location.clone();
        let system = system.clone();
        let update = update.clone();
        move |_| {
            if busy.get() {
                return;
            }
            let (Some(entry), Some(results), Some(status)) =
                (entry.upgrade(), results.upgrade(), status.upgrade())
            else {
                return;
            };
            let query = entry.text().trim().to_string();
            if query.chars().count() < 2 {
                status.set_text("Enter at least two characters");
                return;
            }
            busy.set(true);
            while let Some(child) = results.first_child() {
                results.remove(&child);
            }
            if let Some(scroll) = results_scroll.upgrade() {
                scroll.set_visible(true);
            }
            if let Some(scroll) = report_scroll.upgrade() {
                scroll.set_visible(false);
            }
            status.set_text("Searching cities…");
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(search(&query));
            });
            let results = results.downgrade();
            let status = status.downgrade();
            let busy = busy.clone();
            let location = location.clone();
            let system = system.clone();
            let update = update.clone();
            glib::MainContext::default().spawn_local(async move {
                let found = rx.recv().await;
                busy.set(false);
                let (Some(results), Some(status)) = (results.upgrade(), status.upgrade()) else {
                    return;
                };
                match found {
                    Ok(Ok(cities)) => {
                        status.set_text(if cities.is_empty() {
                            "No cities found"
                        } else {
                            "Choose a location"
                        });
                        for city in cities {
                            let button = crate::info::button(&city.name);
                            let location = location.clone();
                            let system = system.clone();
                            let update = update.clone();
                            let busy = busy.clone();
                            let weak_results = results.downgrade();
                            let status = status.downgrade();
                            button.connect_clicked(move |_| {
                                if busy.get() {
                                    return;
                                }
                                busy.set(true);
                                let saved = crate::config::save_values(vec![
                                    (
                                        "weather_location".into(),
                                        serde_json::to_value(&city).unwrap(),
                                    ),
                                    ("weather_system".into(), false.into()),
                                ]);
                                let busy = busy.clone();
                                let city = city.clone();
                                let location = location.clone();
                                let system = system.clone();
                                let update = update.clone();
                                let weak_results = weak_results.clone();
                                let status = status.clone();
                                glib::MainContext::default().spawn_local(async move {
                                    let result = saved.await;
                                    busy.set(false);
                                    if let Err(error) = result {
                                        if let Some(status) = status.upgrade() {
                                            status.set_text(&error);
                                        }
                                        return;
                                    }
                                    *location.borrow_mut() = city;
                                    system.set(false);
                                    if let Some(results) = weak_results.upgrade() {
                                        while let Some(child) = results.first_child() {
                                            results.remove(&child);
                                        }
                                    }
                                    update();
                                });
                            });
                            results.append(&button);
                        }
                    }
                    Ok(Err(error)) => status.set_text(&error),
                    Err(_) => status.set_text("Location search stopped"),
                }
            });
        }
    });
    entry.connect_activate({
        let find = find.downgrade();
        move |_| {
            if let Some(find) = find.upgrade() {
                find.emit_clicked();
            }
        }
    });
    update();
    outer
}

#[cfg(test)]
pub fn regression_checks(app: &gtk::Application) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.add_css_class("menu-content");
    content.append(&crate::info::label("Weather · Aktobe", "menu-heading"));
    let report = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.append(&report);
    let data = serde_json::json!({
        "current": { "temperature_2m": 13.0, "apparent_temperature": 10.0, "relative_humidity_2m": 49, "wind_speed_10m": 11, "weather_code": 0, "time": "2026-09-27T10:30" },
        "timezone": "Asia/Aqtobe",
        "daily": { "time": ["2026-09-27", "2026-09-28", "2026-09-29"], "temperature_2m_min": [6, 4, 6], "temperature_2m_max": [18, 19, 21], "weather_code": [1, 3, 3], "precipitation_probability_max": [0, 0, 5] }
    });
    render_report(&report, format_forecast(&data).unwrap());
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .default_width(460)
        .child(&content)
        .build();
    window.present();
    crate::ui_tests::pump(150);
    assert!(
        report
            .first_child()
            .unwrap()
            .has_css_class("weather-current")
    );
    crate::ui_tests::capture("weather");
    window.close();
}

#[cfg(test)]
mod tests {
    #[test]
    fn incomplete_weather_is_not_presented_as_zero() {
        assert!(super::format_forecast(&serde_json::json!({})).is_err());
        let text = super::format_forecast(&serde_json::json!({"current":{"temperature_2m":12.4}}))
            .unwrap();
        assert_eq!(text.temperature, "12°C");
        assert!(text.detail.contains("Humidity —%"));
    }
}
