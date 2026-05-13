//! Typed wrapper over the `settings` key/value table.
//!
//! The configuration screen in `main.rs` persists each `LineEdit` through this
//! module using strongly-typed accessors. Unknown keys round-trip through the
//! generic `get` / `set` helpers.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use super::db::SharedDb;

/// Built-in setting keys. Adding a new setting only requires a new constant
/// (plus a typed accessor if appropriate) - the schema does not change.
pub mod keys {
    pub const RTSP_URL: &str = "client.rtsp_url";
    pub const ROV_HTTP_BASE: &str = "client.rov_http_base";
    pub const ROV_UDP_BIND_HOST: &str = "client.rov_udp_bind_host";
    pub const ROV_UDP_PORT: &str = "client.rov_udp_port";
    pub const OSM_TILE_USER_AGENT: &str = "client.osm_tile_user_agent";
    pub const SERVER_BASE_URL: &str = "server.base_url";
    pub const ROV_NETWORK_INTERFACE: &str = "client.rov_network_interface";
    pub const NMEA_GPS_HOST: &str = "client.nmea_gps_host";
    pub const NMEA_GPS_PORT: &str = "client.nmea_gps_port";
}

/// Persisted configuration accessor.
#[derive(Clone)]
pub struct ConfigStore {
    db: SharedDb,
}

impl ConfigStore {
    pub(crate) fn new(db: SharedDb) -> Self {
        Self { db }
    }

    /// Reads an arbitrary setting. Returns `None` when the key is not set.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.db.lock().expect("settings mutex poisoned");
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("reading setting {key}"))
    }

    /// Reads a setting or returns `default` when it is unset or empty.
    pub fn get_or(&self, key: &str, default: &str) -> Result<String> {
        Ok(self
            .get(key)?
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default.to_owned()))
    }

    /// Upserts a setting. Trimming/validation is the caller's responsibility.
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.db.lock().expect("settings mutex poisoned");
        conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .with_context(|| format!("writing setting {key}"))?;
        Ok(())
    }

    /// Removes a setting row entirely. Missing keys are treated as success.
    pub fn clear(&self, key: &str) -> Result<()> {
        let conn = self.db.lock().expect("settings mutex poisoned");
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
            .with_context(|| format!("clearing setting {key}"))?;
        Ok(())
    }

    /// Convenience bulk read used at app start to hydrate the Slint bindings.
    pub fn load_client(&self, defaults: &ClientConfigDefaults) -> Result<ClientConfig> {
        Ok(ClientConfig {
            rtsp_url: self.get_or(keys::RTSP_URL, defaults.rtsp_url)?,
            rov_http_base: self.get_or(keys::ROV_HTTP_BASE, defaults.rov_http_base)?,
            rov_udp_bind_host: self.get_or(keys::ROV_UDP_BIND_HOST, defaults.rov_udp_bind_host)?,
            rov_udp_port: self.get_or(keys::ROV_UDP_PORT, defaults.rov_udp_port)?,
            osm_tile_user_agent: self
                .get_or(keys::OSM_TILE_USER_AGENT, defaults.osm_tile_user_agent)?,
            server_base_url: self.get_or(keys::SERVER_BASE_URL, defaults.server_base_url)?,
            rov_network_interface: self
                .get_or(keys::ROV_NETWORK_INTERFACE, defaults.rov_network_interface)?,
            nmea_gps_host: self.get_or(keys::NMEA_GPS_HOST, defaults.nmea_gps_host)?,
            nmea_gps_port: self.get_or(keys::NMEA_GPS_PORT, defaults.nmea_gps_port)?,
        })
    }

    /// Persists the current client configuration back to the database.
    pub fn save_client(&self, config: &ClientConfig) -> Result<()> {
        let conn = self.db.lock().expect("settings mutex poisoned");
        let tx = conn.unchecked_transaction()?;
        for (key, value) in [
            (keys::RTSP_URL, config.rtsp_url.as_str()),
            (keys::ROV_HTTP_BASE, config.rov_http_base.as_str()),
            (keys::ROV_UDP_BIND_HOST, config.rov_udp_bind_host.as_str()),
            (keys::ROV_UDP_PORT, config.rov_udp_port.as_str()),
            (
                keys::OSM_TILE_USER_AGENT,
                config.osm_tile_user_agent.as_str(),
            ),
            (keys::SERVER_BASE_URL, config.server_base_url.as_str()),
            (
                keys::ROV_NETWORK_INTERFACE,
                config.rov_network_interface.as_str(),
            ),
            (keys::NMEA_GPS_HOST, config.nmea_gps_host.as_str()),
            (keys::NMEA_GPS_PORT, config.nmea_gps_port.as_str()),
        ] {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Snapshot of the client-facing configuration backing the Slint UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientConfig {
    pub rtsp_url: String,
    pub rov_http_base: String,
    pub rov_udp_bind_host: String,
    pub rov_udp_port: String,
    pub osm_tile_user_agent: String,
    pub server_base_url: String,
    /// Optional network interface name (e.g. `en10`) to bind all ROV
    /// connections to. Uses `IP_BOUND_IF` on macOS and `SO_BINDTODEVICE` on
    /// Linux. Empty means the OS chooses the interface.
    pub rov_network_interface: String,
    /// Phone GPS (NMEA over TCP) host. Empty means disabled.
    pub nmea_gps_host: String,
    /// Phone GPS (NMEA over TCP) port as string. Default `"11123"`.
    pub nmea_gps_port: String,
}

/// Compiled-in defaults used when a setting is missing from the database.
#[derive(Clone, Copy, Debug)]
pub struct ClientConfigDefaults<'a> {
    pub rtsp_url: &'a str,
    pub rov_http_base: &'a str,
    pub rov_udp_bind_host: &'a str,
    pub rov_udp_port: &'a str,
    pub osm_tile_user_agent: &'a str,
    pub server_base_url: &'a str,
    pub rov_network_interface: &'a str,
    pub nmea_gps_host: &'a str,
    pub nmea_gps_port: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::open_in_memory;

    const DEFAULTS: ClientConfigDefaults<'static> = ClientConfigDefaults {
        rtsp_url: "rtsp://default",
        rov_http_base: "http://default",
        rov_udp_bind_host: "0.0.0.0",
        rov_udp_port: "8500",
        osm_tile_user_agent: "ua/0",
        server_base_url: "https://third-eye.marshalling.eu",
        rov_network_interface: "",
        nmea_gps_host: "",
        nmea_gps_port: "11123",
    };

    #[test]
    fn round_trips_a_value() {
        let db = open_in_memory().unwrap();
        let store = ConfigStore::new(db);
        assert_eq!(store.get("missing").unwrap(), None);
        store.set("example", "hello").unwrap();
        assert_eq!(store.get("example").unwrap().as_deref(), Some("hello"));
        store.set("example", "world").unwrap();
        assert_eq!(store.get("example").unwrap().as_deref(), Some("world"));
        store.clear("example").unwrap();
        assert_eq!(store.get("example").unwrap(), None);
    }

    #[test]
    fn load_client_uses_defaults_when_empty() {
        let db = open_in_memory().unwrap();
        let store = ConfigStore::new(db);
        let config = store.load_client(&DEFAULTS).unwrap();
        assert_eq!(config.rtsp_url, DEFAULTS.rtsp_url);
        assert_eq!(config.server_base_url, DEFAULTS.server_base_url);
    }

    #[test]
    fn save_client_persists_all_fields() {
        let db = open_in_memory().unwrap();
        let store = ConfigStore::new(db);
        let cfg = ClientConfig {
            rtsp_url: "rtsp://persisted".into(),
            rov_http_base: "http://cam".into(),
            rov_udp_bind_host: "1.2.3.4".into(),
            rov_udp_port: "9000".into(),
            osm_tile_user_agent: "ua/2".into(),
            server_base_url: "https://api.example".into(),
            rov_network_interface: "en10".into(),
            nmea_gps_host: "192.168.1.100".into(),
            nmea_gps_port: "11123".into(),
        };
        store.save_client(&cfg).unwrap();
        let reloaded = store.load_client(&DEFAULTS).unwrap();
        assert_eq!(reloaded, cfg);
    }
}
