use kdl::{KdlDocument, KdlNode};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug)]
pub struct Binding {
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
    pub key: String,
    pub description: String,
    pub action: String,
    pub shell: bool,
    pub overridden: bool,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub root: PathBuf,
    pub files: BTreeMap<PathBuf, String>,
    pub bindings: Vec<Binding>,
    mod_key: String,
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn document(text: &str) -> Result<KdlDocument, String> {
    text.parse().map_err(|e| format!("Invalid KDL: {e}"))
}

fn string(node: &KdlNode, index: usize) -> Option<&str> {
    node.get(index)?.value().as_string()
}

fn include_path(parent: &Path, name: &str) -> PathBuf {
    if let Some(relative) = name.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(relative)
    } else {
        parent.parent().unwrap_or(Path::new(".")).join(name)
    }
}

pub fn config_path() -> PathBuf {
    if let Some(pid) = std::env::var("NIRI_SOCKET")
        .ok()
        .and_then(|s| s.rsplit('.').nth(1).and_then(|v| v.parse::<u32>().ok()))
    {
        let proc = PathBuf::from(format!("/proc/{pid}"));
        if let Ok(bytes) = std::fs::read(proc.join("cmdline")) {
            let args: Vec<_> = bytes
                .split(|b| *b == 0)
                .map(|b| String::from_utf8_lossy(b))
                .collect();
            let value = args
                .windows(2)
                .find(|a| a[0] == "-c" || a[0] == "--config")
                .map(|a| a[1].to_string())
                .or_else(|| {
                    args.iter()
                        .find_map(|a| a.strip_prefix("--config=").map(str::to_owned))
                });
            if let Some(value) = value {
                return std::fs::read_link(proc.join("cwd"))
                    .unwrap_or_default()
                    .join(value);
            }
        }
        if let Ok(bytes) = std::fs::read(proc.join("environ")) {
            let vars: HashMap<_, _> = bytes
                .split(|b| *b == 0)
                .filter_map(|b| {
                    std::str::from_utf8(b)
                        .ok()?
                        .split_once('=')
                        .map(|(k, v)| (k.to_owned(), v.to_owned()))
                })
                .collect();
            if let Some(value) = vars.get("NIRI_CONFIG").filter(|s| !s.is_empty()) {
                return std::fs::read_link(proc.join("cwd"))
                    .unwrap_or_default()
                    .join(value);
            }
            if let Some(home) = vars
                .get("XDG_CONFIG_HOME")
                .filter(|s| Path::new(s).is_absolute())
                .map(PathBuf::from)
                .or_else(|| {
                    vars.get("HOME")
                        .filter(|home| Path::new(home).is_absolute())
                        .map(|home| PathBuf::from(home).join(".config"))
                })
            {
                let user = home.join("niri/config.kdl");
                return if user.exists() {
                    user
                } else {
                    PathBuf::from("/etc/niri/config.kdl")
                };
            }
        }
    }
    if let Some(value) = std::env::var_os("NIRI_CONFIG").filter(|s| !s.is_empty()) {
        return PathBuf::from(value);
    }
    let user = crate::config::path()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("niri/config.kdl");
    if user.exists() {
        user
    } else {
        PathBuf::from("/etc/niri/config.kdl")
    }
}

impl Catalog {
    pub fn load(root: &Path) -> Result<Self, String> {
        let root = std::fs::canonicalize(root).map_err(|e| io_error(root, e))?;
        let mut catalog = Self {
            root: root.clone(),
            files: BTreeMap::new(),
            bindings: Vec::new(),
            mod_key: "super".into(),
        };
        catalog.visit(&root, &mut Vec::new())?;
        let mut effective = HashMap::new();
        for index in 0..catalog.bindings.len() {
            let key = normalize(&catalog.bindings[index].key, &catalog.mod_key);
            if let Some(previous) = effective.insert(key, index) {
                catalog.bindings[previous].overridden = true;
            }
        }
        Ok(catalog)
    }

    fn visit(&mut self, path: &Path, stack: &mut Vec<PathBuf>) -> Result<(), String> {
        let path = std::fs::canonicalize(path).map_err(|e| io_error(path, e))?;
        if stack.len() >= 64 || stack.contains(&path) {
            return Err(format!("Include cycle or depth limit: {}", path.display()));
        }
        let text =
            crate::storage::read_text(&path, 4 * 1024 * 1024).map_err(|e| io_error(&path, e))?;
        if text.len() > 4 * 1024 * 1024 || self.bindings.len() > 10000 {
            return Err("Niri configuration exceeds the editor size limit".into());
        }
        if self.files.len() >= 256
            || self.files.values().map(String::len).sum::<usize>() + text.len() > 16 * 1024 * 1024
        {
            return Err("Niri configuration exceeds the editor size limit".into());
        }
        let doc = document(&text).map_err(|e| io_error(&path, e))?;
        self.files.insert(path.clone(), text);
        stack.push(path.clone());
        for node in doc.nodes() {
            match node.name().value() {
                "include" => {
                    let name = string(node, 0).ok_or("Include requires a file path")?;
                    let next = include_path(&path, name);
                    let optional =
                        node.get("optional").and_then(|e| e.value().as_bool()) == Some(true);
                    if optional && !next.try_exists().map_err(|e| io_error(&next, e))? {
                        continue;
                    }
                    self.visit(&next, stack)?;
                }
                "input" => {
                    if let Some(value) = node
                        .children()
                        .and_then(|d| d.get("mod-key"))
                        .and_then(|n| string(n, 0))
                    {
                        self.mod_key = value.to_lowercase();
                    }
                }
                "binds" => {
                    for bind in node.children().into_iter().flat_map(|d| d.nodes()) {
                        let span = bind.name().span();
                        let action = bind
                            .children()
                            .map(ToString::to_string)
                            .unwrap_or_default()
                            .trim()
                            .to_owned();
                        let command = bind.children().and_then(|d| d.nodes().first());
                        let shell = command.is_some_and(|n| {
                            n.name().value() == "spawn"
                                && string(n, 0).is_some_and(|s| {
                                    Path::new(s).file_name().is_some_and(|n| n == "chuhshell")
                                })
                        });
                        let description = bind
                            .get("hotkey-overlay-title")
                            .and_then(|e| e.value().as_string())
                            .filter(|s| !s.is_empty())
                            .map(|title| {
                                gtk::pango::parse_markup(title, '\0')
                                    .map(|(_, text, _)| text.to_string())
                                    .unwrap_or_else(|_| title.to_owned())
                            })
                            .unwrap_or_else(|| describe(command, shell));
                        self.bindings.push(Binding {
                            path: path.clone(),
                            start: span.offset(),
                            end: span.offset() + span.len(),
                            key: bind.name().value().into(),
                            description,
                            action,
                            shell,
                            overridden: false,
                        });
                    }
                }
                _ => {}
            }
        }
        stack.pop();
        Ok(())
    }

    pub fn enrich_descriptions(&mut self) {
        let Ok(help) = crate::process::run("niri", &["msg", "action", "--help"]) else {
            return;
        };
        let mut descriptions = HashMap::new();
        let mut action = None;
        for line in help.lines() {
            if line.starts_with("  ") && !line.starts_with("   ") {
                action = Some(line.trim().to_owned());
            } else if line.starts_with("          ")
                && let Some(action) = action.take()
            {
                descriptions.insert(action, line.trim().to_owned());
            }
        }
        for binding in &mut self.bindings {
            if let Ok(doc) = document(&binding.action)
                && let Some(node) = doc.nodes().first()
            {
                let fallback = describe(Some(node), binding.shell);
                if binding.description == fallback
                    && let Some(description) = descriptions.get(node.name().value())
                    && !matches!(node.name().value(), "spawn" | "spawn-sh")
                {
                    let args = arguments(node);
                    binding.description = if args.is_empty() {
                        description.clone()
                    } else {
                        format!("{description} · {args}")
                    };
                }
            }
        }
    }

    pub fn replacement(&self, binding: &Binding, key: &str) -> Result<String, String> {
        validate_key(key)?;
        if normalize(key, &self.mod_key) != normalize(&binding.key, &self.mod_key)
            && self.bindings.iter().any(|other| {
                !(other.path == binding.path && other.start == binding.start)
                    && normalize(&other.key, &self.mod_key) == normalize(key, &self.mod_key)
            })
        {
            return Err(format!(
                "{key} is already assigned in the Niri configuration"
            ));
        }
        let original = self.files.get(&binding.path).ok_or("Unknown source file")?;
        let mut text = original.clone();
        text.replace_range(
            binding.start..binding.end,
            &kdl::KdlIdentifier::from(key).to_string(),
        );
        document(&text)?;
        Ok(text)
    }

    pub fn save(&self, binding: &Binding, key: &str) -> Result<(), String> {
        let replacement = self.replacement(binding, key)?;
        let mut files = self.files.clone();
        files.insert(binding.path.clone(), replacement.clone());
        validate_files(&self.root, &files)?;
        for (path, original) in &self.files {
            if crate::storage::read_text(path, 4 * 1024 * 1024).map_err(|e| io_error(path, e))?
                != *original
            {
                return Err("Configuration changed elsewhere. Refresh before saving.".into());
            }
        }
        atomic_write(&binding.path, &replacement)?;
        Ok(())
    }
}

pub fn installation_plan(
    destination: &Path,
    root: Option<&Path>,
    preserve: bool,
) -> Result<serde_json::Value, String> {
    let root = root.map(Path::to_path_buf).unwrap_or_else(config_path);
    if !root.exists() {
        if root == Path::new("/etc/niri/config.kdl") {
            return Ok(serde_json::json!([]));
        }
        return Err(format!(
            "Niri configuration does not exist: {}",
            root.display()
        ));
    }
    let catalog = Catalog::load(&root)?;
    let mut replacements = BTreeMap::new();
    let startup = "spawn-at-startup \"systemctl\" \"--user\" \"start\" \"chuhshell.service\"\n";
    let startup_doc = document(startup)?;
    let mut has_startup = false;
    for (path, text) in &catalog.files {
        let mut doc = document(text)?;
        let mut changed = false;
        for node in doc.nodes_mut() {
            if !preserve
                && node.name().value() == "binds"
                && let Some(bindings) = node.children_mut()
            {
                let old = bindings.nodes().len();
                bindings.nodes_mut().retain(|binding| {
                    !["Mod+Space", "Mod+C", "Mod+Shift+D"].iter().any(|key| {
                        normalize(key, &catalog.mod_key)
                            == normalize(binding.name().value(), &catalog.mod_key)
                    })
                });
                changed |= old != bindings.nodes().len();
            }
            if node.name().value() == "spawn-at-startup" {
                if string(node, 0) == Some("systemctl")
                    && string(node, 1) == Some("--user")
                    && string(node, 2) == Some("start")
                    && string(node, 3) == Some("chuhshell.service")
                {
                    has_startup = true;
                } else if string(node, 0)
                    .is_some_and(|s| Path::new(s).file_name().is_some_and(|s| s == "chuhshell"))
                    && node.entries().len() == 1
                {
                    *node = startup_doc.nodes()[0].clone();
                    has_startup = true;
                    changed = true;
                }
            }
        }
        replacements.insert(
            path.clone(),
            if changed {
                doc.to_string()
            } else {
                text.clone()
            },
        );
    }
    let text = replacements
        .get_mut(&catalog.root)
        .ok_or("Missing root config")?;
    if !preserve {
        let executable = kdl::KdlValue::String(destination.to_string_lossy().into_owned());
        text.push_str(&format!("\nbinds {{\n    Mod+Space hotkey-overlay-title=\"chuh menu\" {{ spawn {executable} \"menu\"; }}\n    Mod+C hotkey-overlay-title=\"clipboard history\" {{ spawn {executable} \"clipboard\"; }}\n}}\n"));
    }
    if !catalog.bindings.iter().any(|binding| {
        normalize(&binding.key, &catalog.mod_key) == normalize("Ctrl+B", &catalog.mod_key)
    }) {
        let executable = kdl::KdlValue::String(destination.to_string_lossy().into_owned());
        let mut doc = document(text)?;
        let addition = document(&format!(
            "binds {{\n    Ctrl+B hotkey-overlay-title=\"wallpaper manager\" {{ spawn {executable} \"wallpaper\"; }}\n}}\n"
        ))?;
        if let Some(bindings) = doc
            .nodes_mut()
            .iter_mut()
            .find(|node| node.name().value() == "binds")
        {
            if bindings.children().is_none() {
                bindings.set_children(KdlDocument::new());
            }
            bindings
                .children_mut()
                .as_mut()
                .unwrap()
                .nodes_mut()
                .extend(
                    addition.nodes()[0]
                        .children()
                        .unwrap()
                        .nodes()
                        .iter()
                        .cloned(),
                );
        } else {
            doc.nodes_mut().push(addition.nodes()[0].clone());
        }
        *text = doc.to_string();
    }
    if !has_startup {
        text.push_str(&format!("\n{startup}"));
    }
    validate_files(&catalog.root, &replacements)?;
    Ok(serde_json::Value::Array(catalog.files.iter().map(|(path, before)| serde_json::json!({"path": path, "before": before, "text": replacements[path]})).collect()))
}

fn validate_files(root: &Path, files: &BTreeMap<PathBuf, String>) -> Result<(), String> {
    let staging = TemporaryDirectory::new()?;
    let paths: BTreeMap<_, _> = files
        .keys()
        .enumerate()
        .map(|(i, path)| (path.clone(), staging.0.join(format!("{i}.kdl"))))
        .collect();
    for (path, text) in files {
        let mut doc = document(text)?;
        for node in doc.nodes_mut() {
            if node.name().value() == "include"
                && let Some(name) = string(node, 0)
            {
                let original = include_path(path, name);
                let target = std::fs::canonicalize(&original)
                    .ok()
                    .and_then(|p| paths.get(&p))
                    .unwrap_or(&original);
                node.get_mut(0)
                    .ok_or("Missing include path")?
                    .set_value(target.to_string_lossy().into_owned());
                node.get_mut(0).ok_or("Missing include path")?.clear_fmt();
            }
        }
        std::fs::write(&paths[path], doc.to_string()).map_err(|e| e.to_string())?;
    }
    crate::process::run(
        "niri",
        &[
            "validate",
            "--config",
            paths[root].to_str().ok_or("Invalid config path")?,
        ],
    )
    .map_err(|e| format!("Niri rejected this change; no files were changed. {e}"))?;
    Ok(())
}

fn arguments(node: &KdlNode) -> String {
    node.entries()
        .iter()
        .map(|e| {
            e.value()
                .as_string()
                .map(str::to_owned)
                .unwrap_or_else(|| e.to_string().trim().into())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe(node: Option<&KdlNode>, shell: bool) -> String {
    let Some(node) = node else {
        return "No action".into();
    };
    let args = arguments(node);
    if shell {
        let command = string(node, 1).unwrap_or("menu");
        let label = match command {
            "menu" => "Open chuh menu",
            "launcher" => "Open app launcher",
            "clipboard" => "Open clipboard history",
            "wallpaper" => "Open wallpaper manager",
            "manage" => "Manage visible apps",
            "notifications" => "Open notifications",
            "background-apps" => "Open background apps",
            "volume-up" => "Increase volume",
            "volume-down" => "Decrease volume",
            "volume-mute" => "Toggle sound",
            "microphone-mute" => "Toggle microphone",
            "brightness-up" | "brightness-key-up" | "brightness-scroll-up" => "Increase brightness",
            "brightness-down" | "brightness-key-down" | "brightness-scroll-down" => {
                "Decrease brightness"
            }
            _ => command,
        };
        return label.into();
    }
    let name = node.name().value();
    if matches!(name, "spawn" | "spawn-sh") {
        return format!("Run {args}");
    }
    let mut words = name.replace('-', " ");
    if let Some(first) = words.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    if !args.is_empty() {
        words.push_str(&format!(" · {args}"));
    }
    words
}

pub fn validate_key(key: &str) -> Result<(), String> {
    let mut parts: Vec<_> = key.split('+').collect();
    let last = parts.pop().unwrap_or_default();
    if last.is_empty() || key.chars().any(char::is_whitespace) || key.len() > 200 {
        return Err("Use a combination such as Mod+Shift+T or XF86AudioMute".into());
    }
    for part in parts {
        if !matches!(
            part.to_lowercase().as_str(),
            "mod"
                | "ctrl"
                | "control"
                | "shift"
                | "alt"
                | "super"
                | "win"
                | "mod5"
                | "iso_level3_shift"
                | "iso_level5_shift"
        ) {
            return Err(format!("Unknown modifier: {part}"));
        }
    }
    if !last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Use the XKB key name, for example plus or semicolon".into());
    }
    Ok(())
}

fn normalize(key: &str, mod_key: &str) -> String {
    let mut parts: Vec<_> = key.split('+').map(|s| s.to_lowercase()).collect();
    let key = parts.pop().unwrap_or_default();
    for part in &mut parts {
        *part = match part.as_str() {
            "mod" => mod_key,
            "control" => "ctrl",
            "win" => "super",
            "iso_level3_shift" => "mod5",
            other => other,
        }
        .to_owned();
    }
    parts.sort();
    parts.dedup();
    parts.push(key);
    parts.join("+")
}

pub fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    crate::storage::atomic_write(path, text.as_bytes()).map_err(|e| io_error(path, e))
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chuhshell-keybindings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path, text: &str) -> PathBuf {
        let path = root.join("config.kdl");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn installation_plan_handles_inline_bindings_and_includes_without_writing() {
        if crate::process::run("niri", &["--version"]).is_err() {
            return;
        }
        let dir = TemporaryDirectory::new().unwrap();
        let root = dir.0.join("config.kdl");
        let included = dir.0.join("keys.kdl");
        let source = "binds { Mod+Space { spawn \"old\"; }; Mod+C { spawn \"old-clipboard\"; }; Alt+F4 { close-window; }; }\n";
        std::fs::write(&included, source).unwrap();
        std::fs::write(&root, "include \"keys.kdl\"\n").unwrap();
        let plan = installation_plan(Path::new("/usr/bin/chuhshell"), Some(&root), false).unwrap();
        assert_eq!(std::fs::read_to_string(&included).unwrap(), source);
        for entry in plan.as_array().unwrap() {
            let text = entry["text"].as_str().unwrap();
            assert!(!text.contains("old-clipboard"));
            if entry["path"].as_str() == included.to_str() {
                assert!(text.contains("Alt+F4"));
            }
            std::fs::write(entry["path"].as_str().unwrap(), text).unwrap();
        }
        assert!(std::fs::read_to_string(&root).unwrap().contains("Ctrl+B"));
        let next = installation_plan(Path::new("/usr/bin/chuhshell"), Some(&root), true).unwrap();
        assert!(
            next.as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["before"] == entry["text"])
        );
    }

    #[test]
    fn reads_active_nodes_and_changes_only_the_key_token() {
        let dir = TemporaryDirectory::new().unwrap();
        let source = "// keep this\nbinds {\n /-Mod+X { quit; }\n \"Mod+T\" repeat=false hotkey-overlay-title=\"Terminal\" { spawn \"foot\"; }\n Mod+C { spawn \"/usr/bin/chuhshell\" \"clipboard\"; }\n}\n";
        let path = fixture(&dir.0, source);
        let catalog = Catalog::load(&path).unwrap();
        assert_eq!(catalog.bindings.len(), 2);
        let binding = &catalog.bindings[0];
        assert_eq!(binding.key, "Mod+T");
        assert_eq!(binding.description, "Terminal");
        assert_eq!(
            catalog.replacement(binding, "Ctrl+Alt+T").unwrap(),
            source.replacen("\"Mod+T\"", "Ctrl+Alt+T", 1)
        );
        assert!(catalog.bindings[1].shell);
        assert_eq!(catalog.bindings[1].description, "Open clipboard history");
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }

    #[test]
    fn includes_follow_order_and_preserve_the_declaring_file() {
        let dir = TemporaryDirectory::new().unwrap();
        let included = dir.0.join("keys.kdl");
        std::fs::write(
            &included,
            "binds { Mod+T { spawn \"foot\"; }; Mod+WheelScrollDown { focus-workspace-down; }; }\n",
        )
        .unwrap();
        let path = fixture(
            &dir.0,
            "include \"keys.kdl\"\nbinds { Mod+T { spawn \"kitty\"; }; }\ninclude optional=true \"absent.kdl\"\n",
        );
        let catalog = Catalog::load(&path).unwrap();
        assert_eq!(catalog.bindings.len(), 3);
        assert!(catalog.bindings[0].overridden);
        assert!(!catalog.bindings[2].overridden);
        assert_eq!(catalog.bindings[1].path, included);
        assert!(
            catalog
                .replacement(&catalog.bindings[1], "Super+T")
                .is_err()
        );
        let updated = catalog
            .replacement(&catalog.bindings[1], "Mod+MouseBack")
            .unwrap();
        assert!(updated.contains("Mod+MouseBack"));
        assert!(updated.contains("Mod+T"));
    }

    #[test]
    fn include_cycles_and_missing_required_files_are_reported() {
        let dir = TemporaryDirectory::new().unwrap();
        let path = fixture(&dir.0, "include \"config.kdl\"\n");
        assert!(Catalog::load(&path).unwrap_err().contains("cycle"));
        fixture(&dir.0, "include \"absent.kdl\"\n");
        assert!(Catalog::load(&path).is_err());
    }

    #[test]
    fn normalizes_modifier_order_aliases_and_custom_mod_key() {
        assert_eq!(
            normalize("Mod+Shift+T", "alt"),
            normalize("Shift+Alt+t", "alt")
        );
        assert_eq!(
            normalize("Win+Control+T", "super"),
            normalize("Ctrl+Mod+t", "super")
        );
        assert!(validate_key("Mod+T { quit; }").is_err());
        assert!(validate_key("Mod+WheelScrollDown").is_ok());
    }

    #[test]
    fn validated_save_preserves_includes_and_rejects_stale_or_invalid_edits() {
        if crate::process::run("niri", &["--version"]).is_err() {
            return;
        }
        let dir = TemporaryDirectory::new().unwrap();
        let included = dir.0.join("keys.kdl");
        let original = "binds { Mod+T repeat=false { spawn \"foot\"; }; }\n";
        std::fs::write(&included, original).unwrap();
        let root_text = "include \"keys.kdl\"\nbinds { Mod+Q { close-window; }; }\n";
        let root = fixture(&dir.0, root_text);
        let catalog = Catalog::load(&root).unwrap();
        catalog.save(&catalog.bindings[0], "Mod+Y").unwrap();
        assert_eq!(
            std::fs::read_to_string(&included).unwrap(),
            original.replace("Mod+T", "Mod+Y")
        );
        assert_eq!(std::fs::read_to_string(&root).unwrap(), root_text);
        assert!(
            catalog
                .save(&catalog.bindings[0], "Mod+U")
                .unwrap_err()
                .contains("changed elsewhere")
        );
        let catalog = Catalog::load(&root).unwrap();
        assert!(
            catalog
                .save(&catalog.bindings[0], "Mod+NotARealKey")
                .is_err()
        );
        assert!(
            std::fs::read_to_string(&included)
                .unwrap()
                .contains("Mod+Y")
        );
    }
}
