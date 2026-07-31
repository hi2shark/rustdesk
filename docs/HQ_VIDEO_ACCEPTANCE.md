# HQ video release acceptance

This document is the evidence checklist for the desktop HQ video engine. Run it
against a self-hosted ID/relay server. Empty server settings and official/public
RustDesk rendezvous addresses are expected to be rejected before connection.

## Required build and automated evidence

- `cargo test --locked --manifest-path libs/hbb_common/Cargo.toml`
- `cargo test --locked --workspace --no-fail-fast -- --skip test_get_cursor_pos --skip test_get_key_state`
- HQ Flutter tests and analyze from `.github/workflows/flutter-build.yml`
- Windows, macOS, and Linux desktop Flutter builds
- Runtime UI scan:
  `rg -n -i "https?://([^\" ]*\\.)?rustdesk\\.com|rustdesk\\.com" flutter/lib src/ui`
  must return no matches.

Record the commit, client/server platforms, GPU and driver, server addresses,
screen resolution, transport, and whether hardware H.265 is available in every
evidence directory.

## Sampling procedure

For each network scenario, warm up for 15 seconds and then sample for 60
seconds. The helper records `tc` state and samples when an evidence directory is
provided:

```bash
sudo ./scripts/hq-video-netem-test.sh eth0 office evidence/office-1080p
sudo ./scripts/hq-video-netem-test.sh eth0 motion evidence/motion-1080p
sudo ./scripts/hq-video-netem-test.sh eth0 congested evidence/congested
sudo ./scripts/hq-video-netem-test.sh eth0 clean evidence/recovery-clean
sudo ./scripts/hq-video-netem-test.sh eth0 clear
```

During each 60-second window, copy the quality report into
`diagnostics.txt`. Keep screenshots or screen recordings for visual and toolbar
checks. Compute queue P95 from the recorded diagnostic samples.

## Release scenarios

| Scenario | Required result | Evidence |
| --- | --- | --- |
| Office 1080p | Actual VP9, I444, about 30 FPS and 12–20 Mbps; complete a side-by-side text clarity review against normal Best | Network samples, copied diagnostics, A/B screenshot |
| Motion 1080p | Actual H.265 hardware when every viewer and the host support it; sampled FPS at least 55 | Network samples and copied diagnostics |
| Congested | Queue-delay P95 below 150 ms; newest frames remain visible and dropped frames recover with a keyframe/short GOP | Queue samples and diagnostics |
| Recovery | After switching congested to clean, bitrate and FPS reach at least 90% of their targets within 5 seconds | Timestamped before/after diagnostics |
| Fallback policy | Enabled: session uses an available codec and shows the requested/actual reason. Disabled: HQ configuration is rejected with a reason while the traditional/current session remains alive | Two diagnostics and session recording |
| Two viewers | A deliberately slow second viewer drives global degradation; disconnecting it removes pending acknowledgements without a false delay spike | Diagnostics from both viewers and disconnect timestamp |

For every scenario, verify the toolbar state, quality panel and copied report
agree in real time for codec, chroma, hardware, bitrate, queue, QoS and
fallback/rejection reason. Closing HQ from the session toolbar must immediately
clear HQ-only diagnostics and restore traditional QoS.
