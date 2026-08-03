//! Fetch optional OSS capability API without requiring Pro.

use hbb_common::{
    config::{keys, Config},
    log,
    transport::{self, ServerCapabilities},
};
use serde_derive::Deserialize;

#[derive(Debug, Deserialize)]
struct CapabilitiesResponse {
    #[serde(default)]
    tcp_rendezvous: bool,
    #[serde(default)]
    tcp_relay: bool,
    #[serde(default)]
    websocket_rendezvous: bool,
    #[serde(default)]
    websocket_relay: bool,
    #[serde(default)]
    native_websocket: bool,
    #[serde(default)]
    tcp_only_registration: bool,
    #[serde(default)]
    force_relay: bool,
    #[serde(default)]
    endpoints: Option<Endpoints>,
}

#[derive(Debug, Deserialize)]
struct Endpoints {
    #[serde(default)]
    ws_id: String,
    #[serde(default)]
    ws_relay: String,
}

#[derive(Debug, Deserialize)]
struct HttpResponseBody {
    #[serde(default)]
    body: String,
    #[serde(default)]
    status_code: i64,
}

/// Try to load capabilities from `{api-server}/api/v1/capabilities`.
/// Failures are non-fatal — builtin config still works.
pub fn try_refresh_capabilities() {
    let api = Config::get_option(keys::OPTION_API_SERVER);
    if api.is_empty() {
        return;
    }
    let url = format!("{}/api/v1/capabilities", api.trim_end_matches('/'));
    std::thread::spawn(move || match crate::http_request_sync(
        url.clone(),
        "GET".to_owned(),
        None,
        "{}".to_owned(),
    ) {
        Ok(raw) => {
            let body = match serde_json::from_str::<HttpResponseBody>(&raw) {
                Ok(r) if (200..300).contains(&r.status_code) || r.status_code == 0 => {
                    if r.body.is_empty() {
                        raw
                    } else {
                        r.body
                    }
                }
                Ok(r) => {
                    log::debug!("capabilities HTTP status {}", r.status_code);
                    return;
                }
                Err(_) => raw,
            };
            match serde_json::from_str::<CapabilitiesResponse>(&body) {
                Ok(resp) => {
                    let caps = ServerCapabilities {
                        tcp_rendezvous: resp.tcp_rendezvous,
                        tcp_relay: resp.tcp_relay,
                        websocket_rendezvous: resp.websocket_rendezvous,
                        websocket_relay: resp.websocket_relay,
                        native_websocket: resp.native_websocket,
                        tcp_only_registration: resp.tcp_only_registration,
                        force_relay: resp.force_relay,
                    };
                    log::info!("loaded server capabilities from {}: {:?}", url, caps);
                    transport::set_capability_override(caps);
                    if let Some(ep) = resp.endpoints {
                        if !ep.ws_id.is_empty()
                            && Config::get_option(keys::OPTION_WEBSOCKET_ID_SERVER).is_empty()
                        {
                            Config::set_option(
                                keys::OPTION_WEBSOCKET_ID_SERVER.to_owned(),
                                ep.ws_id,
                            );
                        }
                        if !ep.ws_relay.is_empty()
                            && Config::get_option(keys::OPTION_WEBSOCKET_RELAY_SERVER).is_empty()
                        {
                            Config::set_option(
                                keys::OPTION_WEBSOCKET_RELAY_SERVER.to_owned(),
                                ep.ws_relay,
                            );
                        }
                    }
                    if resp.native_websocket
                        && Config::get_option(keys::OPTION_NATIVE_WEBSOCKET).is_empty()
                    {
                        Config::set_option(keys::OPTION_NATIVE_WEBSOCKET.to_owned(), "Y".to_owned());
                    }
                }
                Err(err) => log::debug!("capabilities parse failed: {}", err),
            }
        }
        Err(err) => log::debug!("capabilities fetch failed ({}): {}", url, err),
    });
}
