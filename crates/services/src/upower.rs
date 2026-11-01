use std::collections::HashMap;

use futures_util::{StreamExt, stream};
use zbus::{Connection, OwnedObjectPath, OwnedVariant, fdo::PropertiesProxy, proxy};

#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn device_added(&self, device: OwnedObjectPath) -> zbus::Result<()>;

    #[zbus(signal)]
    fn device_removed(&self, device: OwnedObjectPath) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
trait Device {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn energy_rate(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn capacity(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn energy(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn energy_full(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn energy_full_design(&self) -> zbus::Result<f64>;
}

#[proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<HashMap<String, OwnedVariant>>>;
}

const BATTERY: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Battery {
    pub present: bool,
    pub percentage: f64,
    pub state: u32,
    pub time_to_empty: i64,
    pub time_to_full: i64,
    pub energy_rate: f64,
    pub capacity: f64,
    pub energy: f64,
    pub energy_full: f64,
    pub energy_full_design: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PowerState {
    pub battery: Option<Battery>,
    pub on_battery: bool,
}

async fn find_battery<'a>(
    connection: &Connection,
    upower: &UPowerProxy<'a>,
) -> zbus::Result<Option<DeviceProxy<'static>>> {
    for path in upower.enumerate_devices().await? {
        let device = DeviceProxy::builder(connection).path(path).build().await?;
        if device.kind().await? == BATTERY && device.power_supply().await? {
            return Ok(Some(device));
        }
    }
    Ok(None)
}

async fn read(device: &DeviceProxy<'_>) -> zbus::Result<Battery> {
    Ok(Battery {
        present: device.is_present().await?,
        percentage: device.percentage().await?,
        state: device.state().await?,
        time_to_empty: device.time_to_empty().await?,
        time_to_full: device.time_to_full().await?,
        energy_rate: device.energy_rate().await?,
        capacity: device.capacity().await?,
        energy: device.energy().await?,
        energy_full: device.energy_full().await?,
        energy_full_design: device.energy_full_design().await?,
    })
}

pub async fn watch(callback: impl Fn(PowerState)) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let upower = UPowerProxy::new(&connection).await?;
    let added = upower.receive_device_added().await?.map(|_| true);
    let removed = upower.receive_device_removed().await?.map(|_| true);
    let device = find_battery(&connection, &upower).await?;
    let upower_props = PropertiesProxy::builder(&connection)
        .destination("org.freedesktop.UPower")
        .path("/org/freedesktop/UPower")
        .build()
        .await?
        .receive_properties_changed()
        .await?
        .map(|_| false);
    let manager_events = stream::select(stream::select(added, removed), upower_props).boxed_local();
    let device_events = match &device {
        Some(device) => PropertiesProxy::builder(&connection)
            .destination("org.freedesktop.UPower")
            .path(device.inner().path().to_owned())
            .build()
            .await?
            .receive_properties_changed()
            .await?
            .map(|_| false)
            .boxed_local(),
        None => stream::pending().boxed_local(),
    };
    let mut events = stream::select(manager_events, device_events);
    let mut last = None;
    loop {
        let state = PowerState {
            battery: match &device {
                Some(device) => Some(read(device).await?),
                None => None,
            },
            on_battery: upower.on_battery().await?,
        };
        if last.as_ref() != Some(&state) {
            last = Some(state.clone());
            callback(state);
        }
        let devices_changed = events
            .next()
            .await
            .ok_or_else(|| zbus::Error::Failure("upower stream ended".into()))?;
        if devices_changed {
            return Ok(());
        }
    }
}

pub async fn has_performance(connection: &Connection) -> zbus::Result<bool> {
    let proxy = PowerProfilesProxy::new(connection).await?;
    let profiles = proxy.profiles().await?;
    Ok(profiles.iter().any(|profile| {
        profile
            .get("Profile")
            .and_then(|value| <&str>::try_from(value).ok())
            .is_some_and(|name| name == "performance")
    }))
}

pub async fn apply_profile(plugged: bool) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let performance = has_performance(&connection).await?;
    let profile = if plugged && performance {
        "performance"
    } else {
        "balanced"
    };
    let proxy = PowerProfilesProxy::new(&connection).await?;
    if proxy.active_profile().await? != profile {
        proxy.set_active_profile(profile).await?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profiles {
    pub active: String,
    pub has_performance: bool,
}

async fn read_profiles(proxy: &PowerProfilesProxy<'_>) -> zbus::Result<Profiles> {
    let profiles = proxy.profiles().await?;
    Ok(Profiles {
        active: proxy.active_profile().await?,
        has_performance: profiles.iter().any(|profile| {
            profile
                .get("Profile")
                .and_then(|value| <&str>::try_from(value).ok())
                .is_some_and(|name| name == "performance")
        }),
    })
}

pub async fn watch_profiles(callback: impl Fn(Profiles)) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let proxy = PowerProfilesProxy::new(&connection).await?;
    let mut changes = PropertiesProxy::builder(&connection)
        .destination("net.hadess.PowerProfiles")
        .path("/net/hadess/PowerProfiles")
        .build()
        .await?
        .receive_properties_changed()
        .await?;
    let mut last = None;
    loop {
        let profiles = read_profiles(&proxy).await?;
        if last.as_ref() != Some(&profiles) {
            last = Some(profiles.clone());
            callback(profiles);
        }
        if changes.next().await.is_none() {
            return Err(zbus::Error::Failure("power profiles stream ended".into()));
        }
    }
}

pub async fn set_profile(profile: String) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let proxy = PowerProfilesProxy::new(&connection).await?;
    proxy.set_active_profile(&profile).await
}
