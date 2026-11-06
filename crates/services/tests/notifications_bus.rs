#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use services::notifications::{self, Event, Image, MAX_ACTIONS, MAX_BODY, MAX_SUMMARY};
use zbus::{Connection, OwnedVariant, Variant};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

struct Bus {
    child: Child,
    address: String,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn private_bus() -> Bus {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon is needed for this test");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("bus address");
    Bus {
        child,
        address: line.trim().to_string(),
    }
}

type Hints = HashMap<&'static str, OwnedVariant>;

async fn notify(
    client: &Connection,
    replaces: u32,
    summary: &str,
    body: &str,
    actions: &[&str],
    hints: Hints,
    timeout: i32,
) -> zbus::Result<u32> {
    client
        .call_method(
            Some(NAME),
            PATH,
            Some(NAME),
            "Notify",
            &("test", replaces, "", summary, body, actions, hints, timeout),
        )
        .await?
        .body()
        .deserialize::<u32>()
}

fn image(
    width: i32,
    height: i32,
    stride: i32,
    alpha: bool,
    channels: i32,
    data: Vec<u8>,
) -> OwnedVariant {
    let value = Variant::from((width, height, stride, alpha, 8i32, channels, data));
    OwnedVariant::try_from(value).expect("image")
}

#[derive(Default)]
struct Seen {
    added: Vec<notifications::Notification>,
    updated: Vec<(u32, bool, bool)>,
    closed: Vec<u32>,
}

#[test]
fn notification_server_survives_abuse() {
    let bus = private_bus();
    let seen = Arc::new(Mutex::new(Seen::default()));
    zbus::block_on(async {
        let events = seen.clone();
        let _server = notifications::serve_on(Some(&bus.address), move |event| {
            let mut seen = events.lock().unwrap();
            match event {
                Event::Added(notification) => seen.added.push(notification),
                Event::Updated {
                    notification,
                    summary_changed,
                    body_changed,
                } => seen
                    .updated
                    .push((notification.id, summary_changed, body_changed)),
                Event::Closed(id) => seen.closed.push(id),
            }
        })
        .await
        .expect("serve");
        let client = zbus::connection::Builder::address(bus.address.as_str())
            .build()
            .await
            .expect("client");

        let capabilities = client
            .call_method(Some(NAME), PATH, Some(NAME), "GetCapabilities", &())
            .await
            .expect("capabilities");
        assert!(
            capabilities
                .body()
                .deserialize::<Vec<String>>()
                .unwrap()
                .contains(&"body".to_string())
        );

        let started = Instant::now();
        let mut ids = HashSet::new();
        for index in 0..1000 {
            let id = notify(
                &client,
                0,
                &format!("flood {index}"),
                "",
                &[],
                Hints::new(),
                5000,
            )
            .await
            .expect("notify");
            assert!(id != 0 && ids.insert(id), "id {id} reused");
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "1000 notifications took {:?}",
            started.elapsed()
        );

        let first = notify(&client, 0, "chain", "0", &[], Hints::new(), -1)
            .await
            .unwrap();
        let mut current = first;
        for step in 1..50 {
            current = notify(
                &client,
                current,
                "chain",
                &step.to_string(),
                &[],
                Hints::new(),
                -1,
            )
            .await
            .unwrap();
            assert_eq!(current, first);
        }
        let missing = notify(&client, 999_999, "ghost", "", &[], Hints::new(), 0)
            .await
            .unwrap();
        assert_ne!(missing, 999_999);

        let huge = "x".repeat(4 << 20);
        let actions: Vec<String> = (0..200).map(|index| format!("a{index}")).collect();
        let actions: Vec<&str> = actions.iter().map(String::as_str).collect();
        let bloated = notify(&client, 0, &huge, &huge, &actions, Hints::new(), 0)
            .await
            .expect("huge notification");

        let mut bogus = Hints::new();
        bogus.insert(
            "image-data",
            image(100_000, 100_000, 400_000, true, 4, Vec::new()),
        );
        bogus.insert("urgency", OwnedVariant::from(7u8));
        let bogus_id = notify(&client, 0, "bogus image", "", &[], bogus, 0)
            .await
            .expect("bogus image");
        let mut lying = Hints::new();
        lying.insert("image-data", image(64, 64, 4, true, 4, vec![0; 64]));
        let lying_id = notify(&client, 0, "short stride", "", &[], lying, 0)
            .await
            .expect("short stride");
        let mut negative = Hints::new();
        negative.insert("image-data", image(-5, 3, -1, false, 3, vec![1; 64]));
        negative.insert(
            "image-path",
            OwnedVariant::try_from(Variant::from("/nowhere.png")).unwrap(),
        );
        let negative_id = notify(&client, 0, "negative", "", &[], negative, 0)
            .await
            .expect("negative image");
        let mut fine = Hints::new();
        fine.insert("image-data", image(2, 2, 6, false, 3, (0..12).collect()));
        let fine_id = notify(
            &client,
            0,
            "fine image",
            "<b>bold</b> &amp; <i>broken",
            &["default", "Open"],
            fine,
            0,
        )
        .await
        .expect("fine image");
        let mut wrong_types = Hints::new();
        wrong_types.insert("image-data", OwnedVariant::from(5u32));
        wrong_types.insert(
            "urgency",
            OwnedVariant::try_from(Variant::from("high")).unwrap(),
        );
        wrong_types.insert("transient", OwnedVariant::from(3u32));
        notify(&client, 0, "wrong hint types", "", &[], wrong_types, 0)
            .await
            .expect("wrong hint types");

        for id in [0, 424_242, first] {
            client
                .call_method(Some(NAME), PATH, Some(NAME), "CloseNotification", &(id,))
                .await
                .expect("close");
        }

        let mut clients = Vec::new();
        for _ in 0..4 {
            clients.push(
                zbus::connection::Builder::address(bus.address.as_str())
                    .build()
                    .await
                    .unwrap(),
            );
        }
        let parallel = futures_util::future::join_all(clients.iter().enumerate().map(
            |(worker, client)| async move {
                let mut ids = Vec::new();
                for index in 0..250 {
                    ids.push(
                        notify(
                            client,
                            0,
                            &format!("{worker}/{index}"),
                            "",
                            &[],
                            Hints::new(),
                            1,
                        )
                        .await
                        .expect("parallel notify"),
                    );
                }
                ids
            },
        ))
        .await;
        let parallel: Vec<u32> = parallel.into_iter().flatten().collect();
        let unique: HashSet<u32> = parallel.iter().copied().collect();
        assert_eq!(unique.len(), 1000, "parallel ids collided");

        let info = client
            .call_method(Some(NAME), PATH, Some(NAME), "GetServerInformation", &())
            .await
            .expect("still answering");
        let (name, ..) = info
            .body()
            .deserialize::<(String, String, String, String)>()
            .unwrap();
        assert!(!name.is_empty());

        let seen = seen.lock().unwrap();
        assert_eq!(seen.added.len(), 1000 + 1 + 1 + 1 + 4 + 1 + 1000);
        assert_eq!(seen.updated.len(), 49);
        assert!(
            seen.updated
                .iter()
                .all(|(id, summary, body)| *id == first && !summary && *body)
        );
        assert_eq!(seen.closed, vec![first]);
        let find = |id: u32| seen.added.iter().find(|n| n.id == id).expect("added");
        let bloated = find(bloated);
        assert_eq!(bloated.summary.chars().count(), MAX_SUMMARY + 1);
        assert_eq!(bloated.body.chars().count(), MAX_BODY + 1);
        assert_eq!(bloated.actions.len(), MAX_ACTIONS);
        assert!(find(bogus_id).image.is_none());
        assert!(find(lying_id).image.is_none());
        assert!(matches!(find(negative_id).image, Some(Image::Path(_))));
        let Some(Image::Pixels {
            width,
            height,
            rgba,
        }) = &find(fine_id).image
        else {
            panic!("the valid image was dropped");
        };
        assert_eq!((*width, *height, rgba.len()), (2, 2, 16));
        assert_eq!(&rgba[..4], &[0, 1, 2, 255]);
    });
}
