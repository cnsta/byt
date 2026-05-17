//! NetworkManager client via zbus.
//!
//! Exposes:
//! - [`list`] — every WireGuard or OpenVPN connection NM knows about, with state.
//! - [`activate`] / [`deactivate`] — bring a saved connection up/down by id.
//! - [`changes`] — async stream that emits each time NM's state changes.
//!
//! The D-Bus shape of a connection's settings is `a{sa{sv}}`: outer keys are
//! setting group names ("connection", "wireguard", "vpn", "ipv4"…), inner
//! keys are field names within that group. We pull just the few fields we
//! need (`connection.id`, `connection.type`, `vpn.service-type`) and ignore
//! the rest.

use std::collections::{HashMap, HashSet};

use futures::stream::{BoxStream, select_all};
use futures::{Stream, StreamExt};
use zbus::proxy;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

use crate::error::{Error, Result};
use crate::vpn::{Connection, ConnectionState, VpnKind};

const OPENVPN_SERVICE_SUFFIX: &str = ".openvpn";

// dbus proxies

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn deactivate_connection(&self, active_connection: &ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(signal)]
    fn state_changed(&self, state: u32) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(signal)]
    fn new_connection(&self, path: ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    fn connection_removed(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait ConnectionSettings {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
trait ActiveConnection {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn connection(&self) -> zbus::Result<OwnedObjectPath>;
}

// public API

pub async fn list() -> Result<Vec<Connection>> {
    let bus = zbus::Connection::system().await?;
    let nm = NetworkManagerProxy::new(&bus).await?;
    let settings = SettingsProxy::new(&bus).await?;

    let active_obj_paths = active_connection_paths(&bus, &nm).await?;
    let conn_paths = settings.list_connections().await?;

    let mut out = Vec::new();
    for path in conn_paths {
        let cs = ConnectionSettingsProxy::builder(&bus)
            .path(path.clone())?
            .build()
            .await?;
        let s = cs.get_settings().await?;

        let Some(kind) = classify(&s) else { continue };

        let id = get_str(&s, "connection", "id")
            .ok_or_else(|| Error::DBusValue("connection without a `connection.id`".to_owned()))?;

        let state = if active_obj_paths.contains(&path) {
            ConnectionState::Active
        } else {
            ConnectionState::Inactive
        };

        out.push(Connection {
            name: id.to_owned(),
            kind,
            state,
            detail: None,
        });
    }
    Ok(out)
}

pub async fn activate(connection_id: &str) -> Result<()> {
    let bus = zbus::Connection::system().await?;
    let nm = NetworkManagerProxy::new(&bus).await?;
    let settings = SettingsProxy::new(&bus).await?;

    let path = find_path_by_id(&bus, &settings, connection_id).await?;
    let root = ObjectPath::try_from("/").expect("`/` is a valid object path");

    match nm.activate_connection(&path.as_ref(), &root, &root).await {
        Ok(_) => Ok(()),
        Err(err) if looks_like_secrets_error(&err) => Err(Error::SecretsRequired {
            name: connection_id.to_owned(),
        }),
        Err(err) => Err(err.into()),
    }
}

pub async fn deactivate(connection_id: &str) -> Result<()> {
    let bus = zbus::Connection::system().await?;
    let nm = NetworkManagerProxy::new(&bus).await?;

    for ac_path in nm.active_connections().await? {
        let ac = ActiveConnectionProxy::builder(&bus)
            .path(ac_path.clone())?
            .build()
            .await?;
        if ac.id().await? == connection_id {
            nm.deactivate_connection(&ac_path.as_ref()).await?;
            return Ok(());
        }
    }
    Err(Error::NotFound(connection_id.to_owned()))
}

/// Stream that emits `()` whenever something NM-side might have changed.
pub async fn changes() -> Result<BoxStream<'static, ()>> {
    let bus = zbus::Connection::system().await?;
    let nm = NetworkManagerProxy::new(&bus).await?;
    let settings = SettingsProxy::new(&bus).await?;

    let state_changed = nm.receive_state_changed().await?;
    let new_conn = settings.receive_new_connection().await?;
    let conn_removed = settings.receive_connection_removed().await?;
    let active_changed = nm.receive_active_connections_changed().await;

    let streams: Vec<BoxStream<'static, ()>> = vec![
        state_changed.map(|_| ()).boxed(),
        new_conn.map(|_| ()).boxed(),
        conn_removed.map(|_| ()).boxed(),
        active_changed.map(|_| ()).boxed(),
    ];
    Ok(select_all(streams).boxed())
}

// helpers

/// Returns the set of object paths (under `/.../Settings/{N}`) that are
/// currently active. We dereference each ActiveConnection to its underlying
/// settings path so we can match against `list_connections()` results
/// directly.
async fn active_connection_paths(
    bus: &zbus::Connection,
    nm: &NetworkManagerProxy<'_>,
) -> Result<HashSet<OwnedObjectPath>> {
    let mut out = HashSet::new();
    for ac_path in nm.active_connections().await? {
        let ac = ActiveConnectionProxy::builder(bus)
            .path(ac_path)?
            .build()
            .await?;
        // If the active connection vanished between our two calls, just skip it.
        if let Ok(cp) = ac.connection().await {
            out.insert(cp);
        }
    }
    Ok(out)
}

async fn find_path_by_id(
    bus: &zbus::Connection,
    settings: &SettingsProxy<'_>,
    id: &str,
) -> Result<OwnedObjectPath> {
    for path in settings.list_connections().await? {
        let cs = ConnectionSettingsProxy::builder(bus)
            .path(path.clone())?
            .build()
            .await?;
        let s = cs.get_settings().await?;
        if get_str(&s, "connection", "id") == Some(id) {
            return Ok(path);
        }
    }
    Err(Error::NotFound(id.to_owned()))
}

fn classify(settings: &HashMap<String, HashMap<String, OwnedValue>>) -> Option<VpnKind> {
    let conn_type = get_str(settings, "connection", "type")?;
    match conn_type {
        "wireguard" => Some(VpnKind::WireGuard),
        "vpn" => {
            let svc = get_str(settings, "vpn", "service-type")?;
            svc.ends_with(OPENVPN_SERVICE_SUFFIX)
                .then_some(VpnKind::OpenVpn)
        }
        _ => None,
    }
}

fn get_str<'a>(
    settings: &'a HashMap<String, HashMap<String, OwnedValue>>,
    section: &str,
    key: &str,
) -> Option<&'a str> {
    let value = settings.get(section)?.get(key)?;
    <&str>::try_from(value).ok()
}

fn looks_like_secrets_error(err: &zbus::Error) -> bool {
    let msg = err.to_string();
    msg.contains("Secrets") || msg.contains("secret")
}

// public re-export for the change-stream subscription

#[allow(dead_code)]
pub fn into_message_stream<M, F>(stream: BoxStream<'static, ()>, mut f: F) -> impl Stream<Item = M>
where
    F: FnMut() -> M + 'static,
{
    stream.map(move |()| f())
}
