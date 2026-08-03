# OSS native WebSocket / TCP-only (client)

## Transport modes

| Mode | Config | Behavior |
|------|--------|----------|
| Auto | `transport-mode=auto` (default) | Classic UDP/TCP/Relay |
| TCP only | `transport-mode=tcp-only` | No UDP; TCP 21116 signaling; TCP relay; force relay |
| WebSocket only | `transport-mode=websocket-only` | No UDP; WS signaling + WS relay; force relay |

Legacy compatibility:

- `allow-websocket=Y` → treated as WebSocket-only when `transport-mode` unset
- `disable-udp=Y` → treated as TCP-only when `transport-mode` unset

## Enabling against a custom OSS server

Set on the client (options / UI Network):

```text
native-websocket = Y
transport-mode = websocket-only   # or tcp-only
custom-rendezvous-server = your.server
```

Optional explicit endpoints (full URLs, no double ports):

```text
websocket-id-server = wss://remote.example.com/ws/id
websocket-relay-server = wss://remote.example.com/ws/relay
```

Or IP form (no reverse-proxy path):

```text
websocket-id-server = ws://203.0.113.10:21118
websocket-relay-server = ws://203.0.113.10:21119
```

## Capability detection

If `api-server` is set, the client GETs `{api-server}/api/v1/capabilities` and applies:

- `native_websocket` / `tcp_only_registration` / `force_relay`
- optional `endpoints.ws_id` / `endpoints.ws_relay` when local overrides are empty

If the API is unavailable, builtin options still work. **WebSocket is never gated on `is_pro()`.**

## Fallback

When the server rejects TCP/WS registration (`NOT_SUPPORT`):

- Strict modes do **not** silently fall back to UDP
- Set `allow-transport-fallback=Y` only if you explicitly want Auto-style recovery (logged)

## Logs to check

```text
transport mode=...
websocket endpoint: ... -> ...
Transport: TCP Relay / WebSocket Relay
start rendezvous mediator of ... mode=... udp_disabled=...
```

## Known limits

- Domain `check_ws` still maps to `/ws/id` and `/ws/relay` unless overrides are set — put a reverse proxy in front, or use IP:`21118`/`21119`.
- Full desktop `cargo check` may require VCPKG (opus); `hbb_common` unit tests cover transport parsing.
