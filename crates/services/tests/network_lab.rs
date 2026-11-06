#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::Future,
    path::PathBuf,
    pin::pin,
    process::Command,
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

use futures_util::future::{self, Either};
use services::network::{self, Event, Handle, WifiDetails, WifiNetwork, WifiState};

const BUS: &str = "unix:path=/run/chuhshell-lab/bus";
const PASSWORD: &str = "labpass123";

fn netlab(args: &[&str]) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/netlab");
    let status = Command::new("sudo")
        .arg("-n")
        .arg(&script)
        .args(args)
        .status()
        .expect("run tools/netlab");
    assert!(status.success(), "tools/netlab {args:?} failed");
}

fn sleep(duration: Duration) -> impl Future<Output = ()> {
    let (sender, receiver) = services::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        sender.send(());
    });
    async move {
        let _ = receiver.await;
    }
}

enum Reply {
    Password(&'static str),
    Cancel,
}

#[derive(Default)]
struct Shared {
    wifi: WifiState,
    details: Option<WifiDetails>,
    prompts: Vec<String>,
    replies: VecDeque<Reply>,
    changes: u64,
}

struct Lab {
    mode: &'static str,
    handle: Handle,
    shared: Rc<RefCell<Shared>>,
}

impl Lab {
    fn wifi(&self) -> WifiState {
        self.shared.borrow().wifi.clone()
    }

    fn reply(&self, replies: impl IntoIterator<Item = Reply>) {
        self.shared.borrow_mut().replies = replies.into_iter().collect();
    }

    fn prompts(&self) -> Vec<String> {
        self.shared.borrow().prompts.clone()
    }

    fn changes(&self) -> u64 {
        self.shared.borrow().changes
    }

    fn network(&self, name: &str) -> Option<WifiNetwork> {
        self.wifi()
            .networks
            .into_iter()
            .find(|network| network.name == name)
    }

    fn nm(&self) -> bool {
        self.mode == "nm"
    }

    async fn wait(&self, what: &str, seconds: u64, check: impl Fn(&WifiState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            if check(&self.shared.borrow().wifi) {
                return;
            }
            sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "[{}] timed out waiting for {what}; last state: {:#?}",
            self.mode,
            self.wifi()
        );
    }

    async fn scan_for(&self, names: &[&str]) {
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            self.handle.scan().await;
            for _ in 0..40 {
                sleep(Duration::from_millis(100)).await;
                self.handle.refresh_wifi().await;
                let wifi = self.wifi();
                if names.iter().all(|name| {
                    wifi.networks
                        .iter()
                        .any(|network| network.name == *name && network.available)
                }) {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "[{}] scan never found {names:?}: {:#?}",
                self.mode,
                self.wifi()
            );
        }
    }

    async fn connect(&self, name: &str) {
        self.scan_for(&[name]).await;
        let path = self.network(name).expect("network listed").path;
        self.handle.connect(path).await;
    }

    async fn connected_to(&self, name: &str) {
        self.wait(&format!("connected to {name}"), 60, |wifi| {
            wifi.connected && wifi.network == name && !wifi.busy
        })
        .await;
        assert!(
            self.wifi().message.is_empty(),
            "[{}] message after connecting: {}",
            self.mode,
            self.wifi().message
        );
    }

    async fn settled(&self) {
        self.wait("an idle state", 60, |wifi| !wifi.busy).await;
    }
}

async fn drive(lab: &Lab, events: mpsc::Receiver<Event>) {
    loop {
        let mut refresh = false;
        while let Ok(event) = events.try_recv() {
            match event {
                Event::Wifi(state) => lab.shared.borrow_mut().wifi = state,
                Event::Details(details) => lab.shared.borrow_mut().details = Some(details),
                Event::WifiChanged => {
                    lab.shared.borrow_mut().changes += 1;
                    refresh = true;
                }
                Event::WifiPrompt(name) if !name.is_empty() => {
                    let reply = {
                        let mut shared = lab.shared.borrow_mut();
                        shared.prompts.push(name);
                        shared.replies.pop_front()
                    };
                    match reply {
                        Some(Reply::Password(password)) => lab.handle.password(password.into()),
                        Some(Reply::Cancel) | None => lab.handle.cancel(),
                    }
                }
                _ => {}
            }
        }
        if refresh {
            sleep(Duration::from_millis(150)).await;
            lab.handle.refresh_wifi().await;
        } else {
            sleep(Duration::from_millis(30)).await;
        }
    }
}

fn run(mode: &'static str, scenario: impl AsyncFnOnce(&Lab)) {
    assert!(
        std::path::Path::new("/run/chuhshell-lab/bus").exists(),
        "the lab is down: run tools/netlab up"
    );
    netlab(&["reset", mode]);
    zbus::block_on(async {
        let (sender, events) = mpsc::channel();
        let handle = network::start_on(Some(BUS), move |event| {
            let _ = sender.send(event);
        })
        .await
        .expect("connect to the lab bus");
        let lab = Lab {
            mode,
            handle,
            shared: Rc::new(RefCell::new(Shared::default())),
        };
        let watcher = lab.handle.clone();
        let background = future::join(
            async move {
                let _ = watcher.watch().await;
            },
            drive(&lab, events),
        );
        let work = async {
            lab.handle.refresh_wifi().await;
            lab.wait("a wifi backend", 20, |wifi| {
                wifi.backend
                    == if mode == "nm" {
                        "networkmanager"
                    } else {
                        "iwd"
                    }
            })
            .await;
            scenario(&lab).await;
        };
        let limit = sleep(Duration::from_secs(300));
        match future::select(pin!(work), future::select(pin!(background), pin!(limit))).await {
            Either::Left(_) => {}
            Either::Right(_) => panic!("[{mode}] scenario timed out"),
        }
    });
}

fn both(scenario: impl AsyncFn(&Lab)) {
    run("nm", async |lab| scenario(lab).await);
    run("iwd", async |lab| scenario(lab).await);
}

#[test]
#[ignore = "needs tools/netlab up"]
fn scan_lists_visible_networks_with_security() {
    both(async |lab| {
        lab.scan_for(&["lab-open", "lab-wpa2", "lab-sae", "lab-mixed"])
            .await;
        let wifi = lab.wifi();
        let security: HashMap<&str, &str> = wifi
            .networks
            .iter()
            .map(|network| (network.name.as_str(), network.security.as_str()))
            .collect();
        assert_eq!(security["lab-open"], "open");
        assert_eq!(security["lab-wpa2"], "psk");
        assert_eq!(security["lab-sae"], "psk");
        assert_eq!(security["lab-mixed"], "psk");
        assert!(!security.contains_key("lab-hidden"));
        assert!(wifi.networks.iter().all(|network| !network.name.is_empty()));
        assert!(wifi.powered && !wifi.connected);
        assert!(!wifi.device.is_empty());
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn open_network_connects_without_a_prompt() {
    both(async |lab| {
        lab.connect("lab-open").await;
        lab.connected_to("lab-open").await;
        assert!(lab.prompts().is_empty());
        let network = lab.network("lab-open").expect("listed");
        assert!(network.connected && network.saved);
        if lab.nm() {
            lab.handle.details().await;
            sleep(Duration::from_millis(200)).await;
            let details = lab.shared.borrow().details.clone().expect("details");
            assert!(details.connected);
            assert!(details.ip.starts_with("10.77.0."), "{details:?}");
            assert_eq!(details.band, "2.4 GHz");
            assert_eq!(details.channel, 1);
        }
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn secured_networks_connect_with_the_password() {
    both(async |lab| {
        for name in ["lab-wpa2", "lab-sae", "lab-mixed"] {
            let before = lab.prompts().len();
            lab.reply([Reply::Password(PASSWORD)]);
            lab.connect(name).await;
            lab.connected_to(name).await;
            assert_eq!(lab.prompts()[before..], [name.to_string()]);
            assert!(lab.network(name).expect("listed").saved);
        }
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn wrong_password_reports_and_leaves_nothing_saved() {
    both(async |lab| {
        lab.reply([Reply::Password("wrongpass1"), Reply::Cancel]);
        lab.connect("lab-wpa2").await;
        lab.wait("a failure message", 90, |wifi| {
            !wifi.busy && !wifi.connected && !wifi.message.is_empty()
        })
        .await;
        assert!(
            lab.wifi().message.to_lowercase().contains("password"),
            "{}",
            lab.wifi().message
        );
        assert!(!lab.network("lab-wpa2").is_some_and(|network| network.saved));
        lab.reply([Reply::Password(PASSWORD)]);
        lab.connect("lab-wpa2").await;
        lab.connected_to("lab-wpa2").await;
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn cancelled_prompt_leaves_nothing_saved() {
    both(async |lab| {
        lab.reply([Reply::Cancel]);
        lab.connect("lab-wpa2").await;
        lab.settled().await;
        sleep(Duration::from_secs(2)).await;
        let wifi = lab.wifi();
        assert!(!wifi.connected && !wifi.busy);
        assert_eq!(lab.prompts(), ["lab-wpa2"]);
        assert!(!lab.network("lab-wpa2").is_some_and(|network| network.saved));
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn hidden_networks_connect_by_name() {
    both(async |lab| {
        lab.reply([Reply::Password(PASSWORD)]);
        lab.handle.connect_hidden("lab-hidden".into()).await;
        lab.connected_to("lab-hidden").await;
        assert_eq!(lab.prompts(), ["lab-hidden"]);
        lab.handle.disconnect().await;
        lab.wait("disconnect", 30, |wifi| !wifi.connected).await;
        lab.handle.connect_hidden("lab-hidden-open".into()).await;
        lab.connected_to("lab-hidden-open").await;
        assert_eq!(lab.prompts(), ["lab-hidden"]);
        lab.handle.connect_hidden("lab-nowhere".into()).await;
        lab.wait("a not found message", 60, |wifi| {
            !wifi.busy && !wifi.message.is_empty()
        })
        .await;
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn saved_network_keeps_its_password_through_autoconnect_and_forget() {
    both(async |lab| {
        lab.reply([Reply::Password(PASSWORD)]);
        lab.connect("lab-wpa2").await;
        lab.connected_to("lab-wpa2").await;
        let known = lab.network("lab-wpa2").expect("listed").known;
        assert!(!known.is_empty());
        lab.handle.autoconnect(known.clone(), false).await;
        lab.wait("autoconnect off", 20, |wifi| {
            wifi.networks
                .iter()
                .any(|network| network.name == "lab-wpa2" && !network.autoconnect)
        })
        .await;
        lab.handle.disconnect().await;
        lab.wait("disconnect", 30, |wifi| !wifi.connected).await;
        sleep(Duration::from_secs(3)).await;
        assert!(!lab.wifi().connected, "autoconnect is off");
        lab.connect("lab-wpa2").await;
        lab.connected_to("lab-wpa2").await;
        assert_eq!(lab.prompts(), ["lab-wpa2"]);
        let known = lab.network("lab-wpa2").expect("listed").known;
        lab.handle.forget(known).await;
        lab.wait("forgotten", 30, |wifi| {
            !wifi.connected
                && !wifi
                    .networks
                    .iter()
                    .any(|network| network.name == "lab-wpa2" && network.saved)
        })
        .await;
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn power_turns_wifi_off_and_on() {
    both(async |lab| {
        lab.connect("lab-open").await;
        lab.connected_to("lab-open").await;
        lab.handle.wifi_power(false).await;
        lab.wait("power off", 30, |wifi| !wifi.powered && !wifi.connected)
            .await;
        assert!(lab.wifi().networks.iter().all(|network| !network.available));
        lab.handle.wifi_power(true).await;
        lab.wait("power on", 30, |wifi| wifi.powered).await;
        lab.scan_for(&["lab-open", "lab-wpa2"]).await;
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn daemon_restart_keeps_the_password_agent() {
    both(async |lab| {
        lab.connect("lab-open").await;
        lab.connected_to("lab-open").await;
        netlab(&["restart"]);
        sleep(Duration::from_secs(1)).await;
        lab.handle.refresh_wifi().await;
        lab.wait("the backend after a restart", 30, |wifi| {
            !wifi.backend.is_empty() && wifi.powered
        })
        .await;
        lab.reply([Reply::Password(PASSWORD)]);
        lab.connect("lab-sae").await;
        lab.connected_to("lab-sae").await;
        assert_eq!(lab.prompts(), ["lab-sae"]);
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn backend_follows_the_running_daemon() {
    run("nm", async |lab| {
        netlab(&["stop"]);
        lab.wait("no backend", 30, |wifi| {
            wifi.backend.is_empty() && !wifi.connected && wifi.state == "unavailable"
        })
        .await;
        netlab(&["use", "iwd"]);
        lab.wait("iwd", 30, |wifi| wifi.backend == "iwd").await;
        lab.reply([Reply::Password(PASSWORD)]);
        lab.connect("lab-wpa2").await;
        lab.connected_to("lab-wpa2").await;
        netlab(&["use", "nm"]);
        lab.wait("networkmanager", 30, |wifi| {
            wifi.backend == "networkmanager"
        })
        .await;
        lab.reply([Reply::Password(PASSWORD)]);
        lab.connect("lab-mixed").await;
        lab.connected_to("lab-mixed").await;
        assert_eq!(lab.prompts(), ["lab-wpa2", "lab-mixed"]);
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn agent_ignores_callers_other_than_networkmanager() {
    run("nm", async |lab| {
        let name = lab.handle.unique_name().expect("unique name");
        let stranger = zbus::connection::Builder::address(BUS)
            .build()
            .await
            .expect("second connection");
        let ssid =
            zbus::OwnedVariant::try_from(zbus::Variant::from(b"evil".to_vec())).expect("ssid");
        let settings: HashMap<String, HashMap<String, zbus::OwnedVariant>> = HashMap::from([(
            "802-11-wireless".to_string(),
            HashMap::from([("ssid".to_string(), ssid)]),
        )]);
        let path =
            zbus::ObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/9").expect("path");
        let reply = stranger
            .call_method(
                Some(name.as_str()),
                "/org/freedesktop/NetworkManager/SecretAgent",
                Some("org.freedesktop.NetworkManager.SecretAgent"),
                "GetSecrets",
                &(
                    settings,
                    path,
                    "802-11-wireless-security",
                    Vec::<String>::new(),
                    1u32,
                ),
            )
            .await;
        assert!(reply.is_err(), "a stranger got secrets");
        sleep(Duration::from_millis(500)).await;
        assert!(lab.prompts().is_empty(), "a stranger opened the prompt");
    });
}

#[test]
#[ignore = "needs tools/netlab up"]
fn connected_idle_stays_quiet() {
    both(async |lab| {
        lab.connect("lab-open").await;
        lab.connected_to("lab-open").await;
        sleep(Duration::from_secs(30)).await;
        let before = lab.changes();
        sleep(Duration::from_secs(30)).await;
        let changes = lab.changes() - before;
        assert!(
            changes <= 1,
            "[{}] {changes} refreshes in 30 s idle",
            lab.mode
        );
    });
}
