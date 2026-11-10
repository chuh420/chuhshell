use std::{
    collections::{BTreeMap, BTreeSet},
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

pub const PROGRAM: &str = "protonvpn";
pub const INTERFACE: &str = "proton0";

const SECURE_CORE: i64 = 1;
const TOR: i64 = 2;
const P2P: i64 = 4;
const STREAMING: i64 = 8;

const COUNTRIES: &[(&str, &str)] = &[
    ("AF", "Afghanistan"),
    ("AL", "Albania"),
    ("DZ", "Algeria"),
    ("AD", "Andorra"),
    ("AO", "Angola"),
    ("AR", "Argentina"),
    ("AM", "Armenia"),
    ("AU", "Australia"),
    ("AT", "Austria"),
    ("AZ", "Azerbaijan"),
    ("BH", "Bahrain"),
    ("BD", "Bangladesh"),
    ("BY", "Belarus"),
    ("BE", "Belgium"),
    ("BT", "Bhutan"),
    ("BO", "Bolivia"),
    ("BA", "Bosnia and Herzegovina"),
    ("BR", "Brazil"),
    ("BN", "Brunei"),
    ("BG", "Bulgaria"),
    ("KH", "Cambodia"),
    ("CM", "Cameroon"),
    ("CA", "Canada"),
    ("TD", "Chad"),
    ("CL", "Chile"),
    ("CN", "China"),
    ("CO", "Colombia"),
    ("KM", "Comoros"),
    ("CR", "Costa Rica"),
    ("HR", "Croatia"),
    ("CU", "Cuba"),
    ("CY", "Cyprus"),
    ("CZ", "Czech Republic"),
    ("CD", "Democratic Republic of the Congo"),
    ("DK", "Denmark"),
    ("DO", "Dominican Republic"),
    ("EC", "Ecuador"),
    ("EG", "Egypt"),
    ("SV", "El Salvador"),
    ("ER", "Eritrea"),
    ("EE", "Estonia"),
    ("ET", "Ethiopia"),
    ("FI", "Finland"),
    ("FR", "France"),
    ("GA", "Gabon"),
    ("GE", "Georgia"),
    ("DE", "Germany"),
    ("GH", "Ghana"),
    ("GR", "Greece"),
    ("GL", "Greenland"),
    ("GT", "Guatemala"),
    ("GN", "Guinea"),
    ("HT", "Haiti"),
    ("HN", "Honduras"),
    ("HK", "Hong Kong"),
    ("HU", "Hungary"),
    ("IS", "Iceland"),
    ("IN", "India"),
    ("ID", "Indonesia"),
    ("IQ", "Iraq"),
    ("IE", "Ireland"),
    ("IL", "Israel"),
    ("IT", "Italy"),
    ("CI", "Ivory Coast"),
    ("JM", "Jamaica"),
    ("JP", "Japan"),
    ("JO", "Jordan"),
    ("KZ", "Kazakhstan"),
    ("KE", "Kenya"),
    ("XK", "Kosovo"),
    ("KW", "Kuwait"),
    ("KG", "Kyrgyzstan"),
    ("LA", "Laos"),
    ("LV", "Latvia"),
    ("LB", "Lebanon"),
    ("LY", "Libya"),
    ("LI", "Liechtenstein"),
    ("LT", "Lithuania"),
    ("LU", "Luxembourg"),
    ("MO", "Macao"),
    ("MK", "Macedonia"),
    ("MY", "Malaysia"),
    ("MT", "Malta"),
    ("MR", "Mauritania"),
    ("MU", "Mauritius"),
    ("MX", "Mexico"),
    ("MD", "Moldova"),
    ("MC", "Monaco"),
    ("MN", "Mongolia"),
    ("ME", "Montenegro"),
    ("MA", "Morocco"),
    ("MZ", "Mozambique"),
    ("MM", "Myanmar"),
    ("NP", "Nepal"),
    ("NL", "Netherlands"),
    ("NZ", "New Zealand"),
    ("NI", "Nicaragua"),
    ("NG", "Nigeria"),
    ("NO", "Norway"),
    ("OM", "Oman"),
    ("PK", "Pakistan"),
    ("PS", "Palestinian Territory"),
    ("PA", "Panama"),
    ("PG", "Papua New Guinea"),
    ("PY", "Paraguay"),
    ("PE", "Peru"),
    ("PH", "Philippines"),
    ("PL", "Poland"),
    ("PT", "Portugal"),
    ("PR", "Puerto Rico"),
    ("QA", "Qatar"),
    ("RO", "Romania"),
    ("RU", "Russia"),
    ("RW", "Rwanda"),
    ("SA", "Saudi Arabia"),
    ("SN", "Senegal"),
    ("RS", "Serbia"),
    ("SG", "Singapore"),
    ("SK", "Slovakia"),
    ("SI", "Slovenia"),
    ("SO", "Somalia"),
    ("ZA", "South Africa"),
    ("KR", "South Korea"),
    ("SS", "South Sudan"),
    ("ES", "Spain"),
    ("LK", "Sri Lanka"),
    ("SD", "Sudan"),
    ("SE", "Sweden"),
    ("CH", "Switzerland"),
    ("SY", "Syria"),
    ("TW", "Taiwan"),
    ("TJ", "Tajikistan"),
    ("TZ", "Tanzania"),
    ("TH", "Thailand"),
    ("TG", "Togo"),
    ("TN", "Tunisia"),
    ("TR", "Turkey"),
    ("TM", "Turkmenistan"),
    ("UG", "Uganda"),
    ("UA", "Ukraine"),
    ("AE", "United Arab Emirates"),
    ("UK", "United Kingdom"),
    ("US", "United States"),
    ("UY", "Uruguay"),
    ("UZ", "Uzbekistan"),
    ("VE", "Venezuela"),
    ("VN", "Vietnam"),
    ("YE", "Yemen"),
    ("ZW", "Zimbabwe"),
];

pub fn available() -> bool {
    crate::programs::exists(PROGRAM)
}

pub fn country_name(code: &str) -> String {
    let upper = code.to_ascii_uppercase();
    COUNTRIES
        .iter()
        .find(|(known, _)| *known == upper)
        .map_or(upper, |(_, name)| (*name).to_string())
}

pub fn proton_dir() -> PathBuf {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| config::paths().home.join(".cache"));
    cache.join("Proton").join("VPN")
}

fn persistence_path() -> PathBuf {
    proton_dir()
        .join("connection")
        .join("connection_persistence.json")
}

fn serverlist_path() -> PathBuf {
    proton_dir().join("serverlist.json")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connection {
    pub server: String,
    pub ip: String,
    pub domain: String,
    pub protocol: String,
    pub since: u64,
}

#[derive(Deserialize)]
struct RawPersistence {
    protocol: String,
    server: RawPersistedServer,
}

#[derive(Deserialize)]
struct RawPersistedServer {
    #[serde(default)]
    server_ip: String,
    #[serde(default)]
    domain: String,
    server_name: String,
}

pub fn parse_connection(text: &str) -> Option<Connection> {
    let raw: RawPersistence = serde_json::from_str(text).ok()?;
    if raw.server.server_name.is_empty() {
        return None;
    }
    Some(Connection {
        server: raw.server.server_name,
        ip: raw.server.server_ip,
        domain: raw.server.domain,
        protocol: raw.protocol,
        since: 0,
    })
}

fn unix(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub fn now() -> u64 {
    unix(SystemTime::now())
}

pub fn persistence_stamp() -> Option<u64> {
    let meta = std::fs::metadata(persistence_path()).ok()?;
    Some(meta.modified().map_or(1, unix))
}

pub fn read_connection() -> Option<Connection> {
    let path = persistence_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let mut connection = parse_connection(&text)?;
    connection.since = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .map_or(0, unix);
    Some(connection)
}

pub fn link_up() -> bool {
    Path::new("/sys/class/net").join(INTERFACE).exists()
}

pub fn traffic() -> Option<(u64, u64)> {
    let base = Path::new("/sys/class/net")
        .join(INTERFACE)
        .join("statistics");
    let read = |name: &str| -> Option<u64> {
        std::fs::read_to_string(base.join(name))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

pub fn protocol_label(protocol: &str) -> String {
    match protocol.to_ascii_lowercase().as_str() {
        "wireguard" => "WireGuard".into(),
        "openvpn-udp" => "OpenVPN UDP".into(),
        "openvpn-tcp" => "OpenVPN TCP".into(),
        "pro-tun" | "protun" => "Stealth".into(),
        "" => "unknown".into(),
        other => other.into(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawList {
    #[serde(default)]
    logical_servers: Vec<RawServer>,
    #[serde(default)]
    max_tier: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawServer {
    #[serde(default)]
    name: String,
    #[serde(default)]
    tier: i64,
    #[serde(default)]
    features: i64,
    #[serde(default)]
    entry_country: String,
    #[serde(default)]
    exit_country: String,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    load: i64,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    status: serde_json::Value,
}

impl RawServer {
    fn enabled(&self) -> bool {
        match &self.status {
            serde_json::Value::Bool(value) => *value,
            serde_json::Value::Number(value) => value.as_i64().unwrap_or(0) != 0,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Country {
    pub code: String,
    pub name: String,
    pub servers: u32,
    pub online: u32,
    pub cities: u32,
    pub load: u32,
    pub best_load: u32,
    pub score: Option<f64>,
    pub p2p: bool,
    pub streaming: bool,
    pub secure_core: bool,
    pub tor: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerInfo {
    pub name: String,
    pub country: String,
    pub entry_country: String,
    pub city: String,
    pub load: u32,
    pub features: i64,
}

impl ServerInfo {
    pub fn tags(&self) -> Vec<&'static str> {
        let mut tags = Vec::new();
        if self.features & SECURE_CORE != 0 {
            tags.push("secure core");
        }
        if self.features & TOR != 0 {
            tags.push("tor");
        }
        if self.features & P2P != 0 {
            tags.push("p2p");
        }
        if self.features & STREAMING != 0 {
            tags.push("streaming");
        }
        tags
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub countries: Vec<Country>,
    pub tier: i64,
    pub servers: u32,
    pub updated: u64,
    pub current: Option<ServerInfo>,
}

impl Catalog {
    pub fn fastest(&self, exclude: &[String]) -> Option<&Country> {
        self.countries
            .iter()
            .filter(|country| country.score.is_some())
            .filter(|country| {
                !exclude
                    .iter()
                    .any(|code| code.eq_ignore_ascii_case(&country.code))
            })
            .min_by(|a, b| {
                a.score
                    .unwrap_or(f64::MAX)
                    .total_cmp(&b.score.unwrap_or(f64::MAX))
            })
    }
}

fn summarize(raw: RawList, current: Option<&str>) -> Catalog {
    let tier = raw.max_tier.unwrap_or(0);
    let mut groups: BTreeMap<String, (Country, BTreeSet<String>, u64)> = BTreeMap::new();
    let mut found = None;
    let total = raw.logical_servers.len() as u32;
    for server in &raw.logical_servers {
        if current.is_some_and(|name| name.eq_ignore_ascii_case(&server.name)) {
            found = Some(ServerInfo {
                name: server.name.clone(),
                country: server.exit_country.to_ascii_uppercase(),
                entry_country: server.entry_country.to_ascii_uppercase(),
                city: server.city.clone().unwrap_or_default(),
                load: server.load.clamp(0, 100) as u32,
                features: server.features,
            });
        }
        let code = server.exit_country.to_ascii_uppercase();
        if code.is_empty() {
            continue;
        }
        let entry = groups.entry(code.clone()).or_insert_with(|| {
            (
                Country {
                    name: country_name(&code),
                    code: code.clone(),
                    best_load: 100,
                    ..Country::default()
                },
                BTreeSet::new(),
                0,
            )
        });
        let (country, cities, load_sum) = entry;
        country.servers += 1;
        country.secure_core |= server.features & SECURE_CORE != 0;
        country.tor |= server.features & TOR != 0;
        let usable =
            server.enabled() && server.tier <= tier && server.features & (SECURE_CORE | TOR) == 0;
        if !usable {
            continue;
        }
        country.online += 1;
        country.p2p |= server.features & P2P != 0;
        country.streaming |= server.features & STREAMING != 0;
        if let Some(city) = server.city.as_ref().filter(|city| !city.is_empty()) {
            cities.insert(city.clone());
        }
        let load = server.load.clamp(0, 100) as u64;
        *load_sum += load;
        country.best_load = country.best_load.min(load as u32);
        if let Some(score) = server.score {
            country.score = Some(country.score.map_or(score, |best| best.min(score)));
        }
    }
    let mut countries: Vec<Country> = groups
        .into_values()
        .map(|(mut country, cities, load_sum)| {
            country.cities = cities.len() as u32;
            if country.online > 0 {
                country.load = (load_sum / country.online as u64) as u32;
            } else {
                country.best_load = 0;
            }
            country
        })
        .collect();
    countries.sort_by(|a, b| a.name.cmp(&b.name));
    Catalog {
        countries,
        tier,
        servers: total,
        updated: 0,
        current: found,
    }
}

pub fn parse_catalog(text: &str, current: Option<&str>) -> Result<Catalog, String> {
    let raw: RawList = serde_json::from_str(text).map_err(|err| err.to_string())?;
    Ok(summarize(raw, current))
}

pub fn serverlist_stamp() -> Option<u64> {
    std::fs::metadata(serverlist_path())
        .and_then(|meta| meta.modified())
        .ok()
        .map(unix)
}

pub fn load_catalog(current: Option<&str>) -> Result<Catalog, String> {
    let path = serverlist_path();
    let file = std::fs::File::open(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let raw: RawList =
        serde_json::from_reader(BufReader::new(file)).map_err(|err| err.to_string())?;
    let mut catalog = summarize(raw, current);
    catalog.updated = serverlist_stamp().unwrap_or(0);
    Ok(catalog)
}

pub fn zone_country(zone_tab: &str, zone: &str) -> Option<String> {
    zone_tab
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split('\t');
            let code = fields.next()?;
            let _ = fields.next()?;
            let name = fields.next()?;
            (name == zone).then(|| code.to_ascii_uppercase())
        })
}

pub fn system_zone() -> Option<String> {
    if let Ok(zone) = std::env::var("TZ") {
        let zone = zone.trim_start_matches(':');
        if !zone.is_empty() && !zone.starts_with('/') {
            return Some(zone.to_string());
        }
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let text = target.to_string_lossy().into_owned();
    let (_, zone) = text.split_once("zoneinfo/")?;
    Some(zone.to_string())
}

pub fn home_country() -> Option<String> {
    let configured = config::current().vpn.home.clone();
    if let Some(code) = configured.filter(|code| !code.trim().is_empty()) {
        return Some(code.trim().to_ascii_uppercase());
    }
    let zone = system_zone()?;
    let table = std::fs::read_to_string("/usr/share/zoneinfo/zone.tab").ok()?;
    zone_country(&table, &zone)
}

pub fn excluded() -> Vec<String> {
    let mut codes: Vec<String> = config::current()
        .vpn
        .exclude
        .iter()
        .map(|code| code.trim().to_ascii_uppercase())
        .filter(|code| !code.is_empty())
        .collect();
    if let Some(home) = home_country()
        && !codes.contains(&home)
    {
        codes.insert(0, home);
    }
    codes
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Fastest,
    Country(String),
    Server(String),
    Random,
    P2p,
    SecureCore,
    Tor,
}

impl Target {
    pub fn args(&self) -> Vec<String> {
        let mut args = vec!["connect".to_string()];
        match self {
            Target::Fastest => {}
            Target::Country(code) => args.extend(["--country".into(), code.clone()]),
            Target::Server(name) => args.push(name.clone()),
            Target::Random => args.push("--random".into()),
            Target::P2p => args.push("--p2p".into()),
            Target::SecureCore => args.push("--securecore".into()),
            Target::Tor => args.push("--tor".into()),
        }
        args
    }

    pub fn describe(&self) -> String {
        match self {
            Target::Fastest => "the fastest server".into(),
            Target::Country(code) => country_name(code),
            Target::Server(name) => name.clone(),
            Target::Random => "a random server".into(),
            Target::P2p => "the fastest P2P server".into(),
            Target::SecureCore => "the fastest Secure Core server".into(),
            Target::Tor => "the fastest Tor server".into(),
        }
    }
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn last_line(text: &str) -> String {
    strip_ansi(text)
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with("Server list is outdated"))
        .unwrap_or_default()
        .to_string()
}

pub fn run(args: &[String]) -> Result<String, String> {
    let output = Command::new(PROGRAM)
        .args(args)
        .stdin(Stdio::null())
        .env("NO_COLOR", "1")
        .output()
        .map_err(|err| format!("{PROGRAM}: {err}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let failed_text = strip_ansi(&stdout).lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("Error") || line.starts_with("Failed")
    });
    if output.status.success() && !failed_text {
        Ok(strip_ansi(&stdout))
    } else {
        let message = [last_line(&stderr), last_line(&stdout)]
            .into_iter()
            .find(|line| !line.is_empty())
            .unwrap_or_else(|| format!("{PROGRAM} exited with {}", output.status));
        Err(message)
    }
}

pub fn new_ip(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.split("IP address is ").nth(1)?;
        let ip = rest.trim().trim_end_matches('.');
        (!ip.is_empty()).then(|| ip.to_string())
    })
}

pub const SETTINGS: &[(&str, &str, &[&str])] = &[
    (
        "netshield",
        "NetShield",
        &["off", "malware-only", "malware-ads-trackers"],
    ),
    ("kill-switch", "Kill switch", &["off", "standard"]),
    ("vpn-accelerator", "Accelerator", &["off", "on"]),
    ("moderate-nat", "Moderate NAT", &["off", "on"]),
    ("port-forwarding", "Port forwarding", &["off", "on"]),
    ("ipv6", "IPv6", &["off", "on"]),
];

pub fn parse_settings(text: &str) -> BTreeMap<String, String> {
    let known: BTreeSet<&str> = SETTINGS.iter().map(|(key, _, _)| *key).collect();
    strip_ansi(text)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let value = parts.next()?;
            (known.contains(key) && parts.next().is_none())
                .then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

pub fn next_value(key: &str, current: &str) -> Option<&'static str> {
    let (_, _, values) = SETTINGS.iter().find(|(name, _, _)| *name == key)?;
    let index = values.iter().position(|value| *value == current);
    Some(values[index.map_or(0, |index| (index + 1) % values.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
        "MaxTier": 2,
        "LogicalServers": [
            {"Name": "KZ#1", "Tier": 2, "Features": 0, "EntryCountry": "KZ", "ExitCountry": "KZ", "City": "Astana", "Load": 10, "Score": 1.5, "Status": true, "Servers": [{"EntryIP": "1.1.1.1"}]},
            {"Name": "AZ#27", "Tier": 2, "Features": 12, "EntryCountry": "AZ", "ExitCountry": "AZ", "City": "Baku", "Load": 0, "Score": 2.9, "Status": true},
            {"Name": "AZ#30", "Tier": 2, "Features": 0, "EntryCountry": "AZ", "ExitCountry": "AZ", "City": "Baku", "Load": 40, "Score": 3.1, "Status": 1},
            {"Name": "SE-AZ#1", "Tier": 2, "Features": 1, "EntryCountry": "SE", "ExitCountry": "AZ", "City": "Baku", "Load": 5, "Score": 0.1, "Status": true},
            {"Name": "RU#5", "Tier": 2, "Features": 0, "EntryCountry": "RU", "ExitCountry": "RU", "City": "Moscow", "Load": 20, "Score": 2.0, "Status": true},
            {"Name": "NL#1", "Tier": 2, "Features": 0, "EntryCountry": "NL", "ExitCountry": "NL", "City": "Amsterdam", "Load": 70, "Score": 5.0, "Status": false},
            {"Name": "US-FREE#2", "Tier": 0, "Features": 16, "EntryCountry": "US", "ExitCountry": "US", "City": "Los Angeles", "Load": 52, "Score": 4.9, "Status": true},
            {"Name": "US-NY#3", "Tier": 3, "Features": 0, "EntryCountry": "US", "ExitCountry": "US", "City": "New York", "Load": 1, "Score": 0.2, "Status": true}
        ]
    }"#;

    #[test]
    fn summarizes_countries() {
        let catalog = parse_catalog(LIST, Some("az#27")).expect("valid list");
        assert_eq!(catalog.tier, 2);
        assert_eq!(catalog.servers, 8);
        let az = catalog
            .countries
            .iter()
            .find(|country| country.code == "AZ")
            .expect("az");
        assert_eq!(az.name, "Azerbaijan");
        assert_eq!(az.servers, 3);
        assert_eq!(az.online, 2);
        assert_eq!(az.cities, 1);
        assert_eq!(az.load, 20);
        assert_eq!(az.best_load, 0);
        assert_eq!(az.score, Some(2.9));
        assert!(az.p2p && az.streaming && az.secure_core && !az.tor);
        let nl = catalog
            .countries
            .iter()
            .find(|country| country.code == "NL")
            .expect("nl");
        assert_eq!(nl.online, 0);
        assert_eq!(nl.score, None);
        let us = catalog
            .countries
            .iter()
            .find(|country| country.code == "US")
            .expect("us");
        assert_eq!(us.online, 1);
        assert_eq!(us.score, Some(4.9));
        let current = catalog.current.expect("current server");
        assert_eq!(current.city, "Baku");
        assert_eq!(current.tags(), vec!["p2p", "streaming"]);
        let names: Vec<&str> = catalog.countries.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(names, vec!["AZ", "KZ", "NL", "RU", "US"]);
    }

    #[test]
    fn fastest_skips_excluded_countries() {
        let catalog = parse_catalog(LIST, None).expect("valid list");
        assert_eq!(catalog.fastest(&[]).map(|c| c.code.as_str()), Some("KZ"));
        let exclude = vec!["kz".to_string(), "RU".to_string()];
        assert_eq!(
            catalog.fastest(&exclude).map(|c| c.code.as_str()),
            Some("AZ")
        );
        assert!(
            parse_catalog("{}", None)
                .expect("empty")
                .fastest(&[])
                .is_none()
        );
        assert!(parse_catalog("[", None).is_err());
    }

    #[test]
    fn parses_persisted_connection() {
        let text = r#"{"connection_id": "abc", "backend": "networkmanager", "protocol": "wireguard",
            "server": {"server_ip": "185.1.2.3", "openvpn_ports": {}, "wireguard_ports": {},
            "domain": "node-az-01.protonvpn.net", "x25519pk": "k", "server_id": "id",
            "server_name": "AZ#27", "has_ipv6_support": true, "label": null}}"#;
        let connection = parse_connection(text).expect("connection");
        assert_eq!(connection.server, "AZ#27");
        assert_eq!(connection.ip, "185.1.2.3");
        assert_eq!(connection.protocol, "wireguard");
        assert_eq!(parse_connection("{}"), None);
        assert_eq!(parse_connection("not json"), None);
        assert_eq!(protocol_label("wireguard"), "WireGuard");
        assert_eq!(protocol_label("openvpn-tcp"), "OpenVPN TCP");
    }

    #[test]
    fn maps_time_zone_to_country() {
        let table = "# comment\nKZ\t+5017+05710\tAsia/Aqtobe\tAqtobe/Aktobe\nRU\t+5545+03735\tEurope/Moscow\tMSK+00\n";
        assert_eq!(zone_country(table, "Asia/Aqtobe"), Some("KZ".into()));
        assert_eq!(zone_country(table, "Europe/Moscow"), Some("RU".into()));
        assert_eq!(zone_country(table, "Asia/Tokyo"), None);
    }

    #[test]
    fn builds_connect_arguments() {
        assert_eq!(Target::Fastest.args(), vec!["connect"]);
        assert_eq!(
            Target::Country("AZ".into()).args(),
            vec!["connect", "--country", "AZ"]
        );
        assert_eq!(Target::SecureCore.args(), vec!["connect", "--securecore"]);
        assert_eq!(Target::Country("de".into()).describe(), "Germany");
        assert_eq!(country_name("xx"), "XX");
    }

    #[test]
    fn reads_cli_output() {
        let output = "Connecting to AZ#27...\nConnected to AZ#27 in Baku, Azerbaijan. Your new IP address is 185.1.2.3.\n";
        assert_eq!(new_ip(output), Some("185.1.2.3".into()));
        assert_eq!(new_ip("Disconnected."), None);
        assert_eq!(
            last_line("\u{1b}[31mServer list is outdated, updating...\nError: no session\n\n"),
            "Error: no session"
        );
    }

    #[test]
    fn parses_settings_table() {
        let text = "Current configuration\nSetting                  Value\n-----------------------  ------------\nnetshield                malware-only\nkill-switch              off\nvpn-accelerator          on\nipv6                     on\n\nUse 'protonvpn config set <setting> <value>' to change settings.\n";
        let settings = parse_settings(text);
        assert_eq!(
            settings.get("netshield").map(String::as_str),
            Some("malware-only")
        );
        assert_eq!(settings.get("kill-switch").map(String::as_str), Some("off"));
        assert_eq!(settings.len(), 4);
        assert_eq!(
            next_value("netshield", "malware-only"),
            Some("malware-ads-trackers")
        );
        assert_eq!(next_value("netshield", "malware-ads-trackers"), Some("off"));
        assert_eq!(next_value("kill-switch", "weird"), Some("off"));
        assert_eq!(next_value("unknown", "on"), None);
    }
}
