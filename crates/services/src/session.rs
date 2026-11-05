use std::process::{Command, Stdio};

use zbus::Connection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    Suspend,
    Reboot,
    PowerOff,
}

impl Power {
    pub fn method(self) -> &'static str {
        match self {
            Power::Suspend => "Suspend",
            Power::Reboot => "Reboot",
            Power::PowerOff => "PowerOff",
        }
    }

    pub fn parse(value: &str) -> Option<Power> {
        match value {
            "suspend" => Some(Power::Suspend),
            "reboot" => Some(Power::Reboot),
            "poweroff" => Some(Power::PowerOff),
            _ => None,
        }
    }
}

pub fn dry_run() -> bool {
    std::env::var_os("CHUHSHELL_DRY_POWER").is_some_and(|value| !value.is_empty())
}

pub async fn power(action: Power) -> zbus::Result<()> {
    if dry_run() {
        log::info!("dry run: logind {}", action.method());
        return Ok(());
    }
    let connection = Connection::system().await?;
    connection
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            action.method(),
            &(true,),
        )
        .await
        .map(|_| ())
}

pub fn first_found<'a>(candidates: &[&'a str], exists: impl Fn(&str) -> bool) -> Option<&'a str> {
    candidates.iter().copied().find(|name| exists(name))
}

pub fn lock_command_with(configured: Option<&str>, exists: impl Fn(&str) -> bool) -> String {
    if let Some(command) = configured {
        return command.to_string();
    }
    first_found(&["hyprlock", "swaylock"], exists)
        .map(str::to_string)
        .unwrap_or_else(|| "loginctl lock-session".into())
}

pub fn screenshot_command_with(
    configured: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(command) = configured {
        return Some(command.to_string());
    }
    ["grim", "slurp", "wl-copy"]
        .iter()
        .all(|name| exists(name))
        .then(|| "grim -g \"$(slurp)\" - | wl-copy".to_string())
}

pub fn mixer_command_with(
    configured: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(command) = configured {
        return Some(command.to_string());
    }
    first_found(&["pavucontrol", "pwvucontrol"], exists).map(str::to_string)
}

pub fn lock_command() -> String {
    lock_command_with(
        config::current().commands.lock.as_deref(),
        crate::programs::exists,
    )
}

pub fn screenshot_command() -> Option<String> {
    screenshot_command_with(
        config::current().commands.screenshot.as_deref(),
        crate::programs::exists,
    )
}

pub fn mixer_command() -> Option<String> {
    mixer_command_with(
        config::current().commands.mixer.as_deref(),
        crate::programs::exists,
    )
}

const TERMINALS: &[&str] = &["kitty", "foot", "alacritty", "wezterm", "ghostty"];

pub fn terminal_prefix(terminal: &str) -> Vec<String> {
    let words: Vec<String> = terminal.split_whitespace().map(str::to_string).collect();
    if words.len() != 1 {
        return words;
    }
    let name = words[0].as_str();
    let base = name.rsplit('/').next().unwrap_or(name);
    let extra: &[&str] = match base {
        "alacritty" | "ghostty" | "xterm" | "konsole" => &["-e"],
        "wezterm" => &["start", "--"],
        "gnome-terminal" | "kgx" => &["--"],
        _ => &[],
    };
    let mut prefix = vec![name.to_string()];
    prefix.extend(extra.iter().map(|item| item.to_string()));
    prefix
}

pub fn terminal_with(
    configured: Option<&str>,
    env_terminal: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    if let Some(terminal) = configured.filter(|value| !value.trim().is_empty()) {
        return Some(terminal_prefix(terminal));
    }
    if let Some(terminal) = env_terminal.filter(|value| !value.trim().is_empty())
        && terminal.split_whitespace().next().is_some_and(&exists)
    {
        return Some(terminal_prefix(terminal));
    }
    if exists("xdg-terminal-exec") {
        return Some(vec!["xdg-terminal-exec".into()]);
    }
    first_found(TERMINALS, exists).map(terminal_prefix)
}

pub fn terminal() -> Option<Vec<String>> {
    terminal_with(
        config::current().general.terminal.as_deref(),
        std::env::var("TERMINAL").ok().as_deref(),
        crate::programs::exists,
    )
}

pub fn shell_command(line: &str) -> Command {
    let mut command = if crate::programs::exists("systemd-run") {
        let mut command = Command::new("systemd-run");
        command.args([
            "--user",
            "--scope",
            "--slice=app.slice",
            "--collect",
            "--quiet",
            "--",
            "sh",
            "-c",
        ]);
        command
    } else {
        let mut command = Command::new("sh");
        command.arg("-c");
        command
    };
    command
        .arg(line)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub fn hook_command(hook: &str, args: &[&str]) -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(format!("{hook} \"$@\""))
        .arg("sh")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |name| names.contains(&name)
    }

    #[test]
    fn lock_choice() {
        assert_eq!(
            lock_command_with(Some("my-lock"), has(&["hyprlock"])),
            "my-lock"
        );
        assert_eq!(
            lock_command_with(None, has(&["swaylock", "hyprlock"])),
            "hyprlock"
        );
        assert_eq!(lock_command_with(None, has(&["swaylock"])), "swaylock");
        assert_eq!(lock_command_with(None, has(&[])), "loginctl lock-session");
    }

    #[test]
    fn screenshot_and_mixer_choice() {
        assert_eq!(
            screenshot_command_with(None, has(&["grim", "slurp", "wl-copy"])).as_deref(),
            Some("grim -g \"$(slurp)\" - | wl-copy")
        );
        assert_eq!(screenshot_command_with(None, has(&["grim", "slurp"])), None);
        assert_eq!(
            screenshot_command_with(Some("flameshot gui"), has(&[])).as_deref(),
            Some("flameshot gui")
        );
        assert_eq!(
            mixer_command_with(None, has(&["pavucontrol"])).as_deref(),
            Some("pavucontrol")
        );
        assert_eq!(mixer_command_with(None, has(&[])), None);
    }

    #[test]
    fn terminal_choice() {
        let all = has(&["kitty", "foot", "xdg-terminal-exec", "alacritty"]);
        assert_eq!(
            terminal_with(Some("foot"), None, &all),
            Some(vec!["foot".into()])
        );
        assert_eq!(
            terminal_with(Some("alacritty"), None, &all),
            Some(vec!["alacritty".into(), "-e".into()])
        );
        assert_eq!(
            terminal_with(Some("kitty --single-instance"), None, &all),
            Some(vec!["kitty".into(), "--single-instance".into()])
        );
        assert_eq!(
            terminal_with(None, Some("alacritty"), &all),
            Some(vec!["alacritty".into(), "-e".into()])
        );
        assert_eq!(
            terminal_with(None, Some("missing"), &all),
            Some(vec!["xdg-terminal-exec".into()])
        );
        assert_eq!(
            terminal_with(None, None, has(&["wezterm"])),
            Some(vec!["wezterm".into(), "start".into(), "--".into()])
        );
        assert_eq!(
            terminal_with(None, None, has(&["kitty"])),
            Some(vec!["kitty".into()])
        );
        assert_eq!(terminal_with(None, None, has(&[])), None);
    }

    #[test]
    fn power_names() {
        assert_eq!(Power::parse("suspend"), Some(Power::Suspend));
        assert_eq!(
            Power::parse("poweroff").map(Power::method),
            Some("PowerOff")
        );
        assert_eq!(Power::parse("halt"), None);
    }

    #[test]
    fn hooks_receive_arguments() {
        let command = hook_command("~/bin/sync", &["/w/a.jpg", "/w/a.jpg"]);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            ["-c", "~/bin/sync \"$@\"", "sh", "/w/a.jpg", "/w/a.jpg"]
        );
    }
}
