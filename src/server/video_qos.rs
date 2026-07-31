use super::*;
use scrap::codec::{Quality, BR_BALANCED, BR_BEST, BR_SPEED};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use hbb_common::video_profile::{
    QosState, RateControlMode, VideoProfile, VideoProfileType, VideoRateConfig, BASE_1080P_KBPS,
};

/*
FPS adjust:
a. new user connected =>set to INIT_FPS
b. TestDelay receive => update user's fps according to network delay
    When network delay < DELAY_THRESHOLD_150MS, set minimum fps according to image quality, and increase fps;
    When network delay >= DELAY_THRESHOLD_150MS, set minimum fps according to image quality, and decrease fps;
c. second timeout / TestDelay receive => update real fps to the minimum fps from all users

ratio adjust:
a. user set image quality => update to the maximum ratio of the latest quality
b. 3 seconds timeout => update ratio according to network delay
    When network delay < DELAY_THRESHOLD_150MS, increase ratio, max 150kbps;
    When network delay >= DELAY_THRESHOLD_150MS, decrease ratio;

adjust between FPS and ratio:
    When network delay < DELAY_THRESHOLD_150MS, fps is always higher than the minimum fps, and ratio is increasing;
    When network delay >= DELAY_THRESHOLD_150MS, fps is always lower than the minimum fps, and ratio is decreasing;

delay:
    use delay minus RTT as the actual network delay

HQ mode (when VideoRateConfig is set):
    Uses queue_delay state machine (Stable/ProbeUp/Hold/Congested/Emergency)
    and profile-specific degradation order instead of the legacy ratio path.
*/

// Constants
pub const FPS: u32 = 30;
pub const MIN_FPS: u32 = 1;
pub const MAX_FPS: u32 = 120;
pub const INIT_FPS: u32 = 15;

// Bitrate ratio constants for different quality levels
const BR_MAX: f32 = 40.0; // 2000 * 2 / 100
const BR_MIN: f32 = 0.2;
const BR_MIN_HIGH_RESOLUTION: f32 = 0.1; // For high resolution, BR_MIN is still too high, so we set a lower limit
const MAX_BR_MULTIPLE: f32 = 1.0;

const HISTORY_DELAY_LEN: usize = 2;
const ADJUST_RATIO_INTERVAL: usize = 3; // Adjust quality ratio every 3 seconds
const DYNAMIC_SCREEN_THRESHOLD: usize = 2; // Allow increase quality ratio if encode more than 2 times in one second
const DELAY_THRESHOLD_150MS: u32 = 150; // 150ms is the threshold for good network condition

// HQ QoS queue delay thresholds (ms)
const HQ_QUEUE_PROBE: u32 = 60;
const HQ_QUEUE_HOLD: u32 = 150;
const HQ_QUEUE_EMERGENCY: u32 = 250;

#[derive(Default, Debug, Clone)]
struct UserDelay {
    response_delayed: bool,
    delay_history: VecDeque<u32>,
    fps: Option<u32>,
    rtt_calculator: RttCalculator,
    quick_increase_fps_count: usize,
    increase_fps_count: usize,
}

impl UserDelay {
    fn add_delay(&mut self, delay: u32) {
        self.rtt_calculator.update(delay);
        if self.delay_history.len() > HISTORY_DELAY_LEN {
            self.delay_history.pop_front();
        }
        self.delay_history.push_back(delay);
    }

    // Average delay minus RTT
    fn avg_delay(&self) -> u32 {
        let len = self.delay_history.len();
        if len > 0 {
            let avg_delay = self.delay_history.iter().sum::<u32>() / len as u32;

            // If RTT is available, subtract it from average delay to get actual network latency
            if let Some(rtt) = self.rtt_calculator.get_rtt() {
                if avg_delay > rtt {
                    avg_delay - rtt
                } else {
                    avg_delay
                }
            } else {
                avg_delay
            }
        } else {
            DELAY_THRESHOLD_150MS
        }
    }
}

// User session data structure
#[derive(Default, Debug, Clone)]
struct UserData {
    auto_adjust_fps: Option<u32>, // reserve for compatibility
    custom_fps: Option<u32>,
    quality: Option<(i64, Quality)>, // (time, quality)
    delay: UserDelay,
    record: bool,
    /// When set, enables HQ QoS path for this user.
    rate_config: Option<VideoRateConfig>,
    profile_type: Option<VideoProfileType>,
}

#[derive(Default, Debug, Clone)]
struct DisplayData {
    send_counter: usize, // Number of times encode during period
    support_changing_quality: bool,
}

/// Pending frame enqueue metadata for queue delay estimation.
#[derive(Debug, Clone)]
struct PendingFrame {
    seq: u64,
    enqueue_instant: Instant,
    bytes: u32,
}

// Main QoS controller structure
pub struct VideoQoS {
    fps: u32,
    ratio: f32,
    users: HashMap<i32, UserData>,
    displays: HashMap<String, DisplayData>,
    bitrate_store: u32,
    adjust_ratio_instant: Instant,
    abr_config: bool,
    new_user_instant: Instant,
    // HQ fields
    hq_enabled: bool,
    hq_target_kbps: u32,
    hq_current_kbps: u32,
    hq_state: QosState,
    hq_good_samples: u32,
    hq_freeze_until: Option<Instant>,
    hq_rate: Option<VideoRateConfig>,
    hq_profile: Option<VideoProfileType>,
    frame_seq: u64,
    pending_frames: VecDeque<PendingFrame>,
    last_queue_delay_ms: u32,
    need_keyframe: bool,
    drop_old_frames: bool,
    encoder_fallback_reason: String,
    encoder_hardware: bool,
}

impl Default for VideoQoS {
    fn default() -> Self {
        VideoQoS {
            fps: FPS,
            ratio: BR_BALANCED,
            users: Default::default(),
            displays: Default::default(),
            bitrate_store: 0,
            adjust_ratio_instant: Instant::now(),
            abr_config: true,
            new_user_instant: Instant::now(),
            hq_enabled: false,
            hq_target_kbps: 0,
            hq_current_kbps: 0,
            hq_state: QosState::Stable,
            hq_good_samples: 0,
            hq_freeze_until: None,
            hq_rate: None,
            hq_profile: None,
            frame_seq: 0,
            pending_frames: VecDeque::new(),
            last_queue_delay_ms: 0,
            need_keyframe: false,
            drop_old_frames: false,
            encoder_fallback_reason: String::new(),
            encoder_hardware: false,
        }
    }
}

// Basic functionality
impl VideoQoS {
    // Calculate seconds per frame based on current FPS
    pub fn spf(&self) -> Duration {
        Duration::from_secs_f32(1. / (self.fps() as f32))
    }

    // Get current FPS within valid range
    pub fn fps(&self) -> u32 {
        let fps = self.fps;
        if fps >= MIN_FPS && fps <= MAX_FPS {
            fps
        } else {
            FPS
        }
    }

    // Store bitrate for later use
    pub fn store_bitrate(&mut self, bitrate: u32) {
        self.bitrate_store = bitrate;
    }

    /// Unified HQ/legacy bitrate lookup so `bitrate()` and `current_kbps()` agree.
    fn effective_kbps(&self) -> u32 {
        if self.hq_enabled {
            if self.hq_current_kbps > 0 {
                self.hq_current_kbps
            } else {
                self.hq_target_kbps
            }
        } else {
            self.bitrate_store
        }
    }

    // Get stored bitrate
    pub fn bitrate(&self) -> u32 {
        self.effective_kbps()
    }

    // Get current bitrate ratio with bounds checking
    pub fn ratio(&mut self) -> f32 {
        if self.hq_enabled {
            let kbps = self.effective_kbps();
            return (kbps as f32 / BASE_1080P_KBPS).clamp(BR_MIN_HIGH_RESOLUTION, BR_MAX);
        }
        if self.ratio < BR_MIN_HIGH_RESOLUTION || self.ratio > BR_MAX {
            self.ratio = BR_BALANCED;
        }
        self.ratio
    }

    pub fn hq_rate_config(&self) -> Option<&VideoRateConfig> {
        self.hq_rate.as_ref()
    }

    pub fn hq_enabled(&self) -> bool {
        self.hq_enabled
    }

    pub fn hq_state(&self) -> QosState {
        self.hq_state
    }

    pub fn last_queue_delay_ms(&self) -> u32 {
        self.last_queue_delay_ms
    }

    pub fn current_kbps(&self) -> u32 {
        self.effective_kbps()
    }

    pub fn take_need_keyframe(&mut self) -> bool {
        let v = self.need_keyframe;
        self.need_keyframe = false;
        v
    }

    pub fn take_drop_old_frames(&mut self) -> bool {
        let v = self.drop_old_frames;
        self.drop_old_frames = false;
        v
    }

    pub fn set_encoder_fallback_reason(&mut self, reason: String) {
        self.encoder_fallback_reason = reason;
    }

    pub fn encoder_fallback_reason(&self) -> &str {
        &self.encoder_fallback_reason
    }

    pub fn set_encoder_hardware(&mut self, hw: bool) {
        self.encoder_hardware = hw;
    }

    pub fn is_encoder_hardware(&self) -> bool {
        self.encoder_hardware
    }

    /// Clear pending-frame accounting without driving the QoS state machine.
    pub fn clear_pending_frames(&mut self) {
        self.pending_frames.clear();
    }

    fn pending_frame_limit(&self) -> usize {
        let fps = self.fps().max(1) as u64;
        let max_queue_ms = self
            .hq_rate
            .as_ref()
            .map(|r| r.max_queue_ms as u64)
            .unwrap_or(150);
        ((fps * max_queue_ms) / 1000).clamp(30, 120) as usize
    }

    pub fn on_frame_enqueue(&mut self, bytes: u32) -> u64 {
        self.frame_seq = self.frame_seq.wrapping_add(1);
        let seq = self.frame_seq;
        self.pending_frames.push_back(PendingFrame {
            seq,
            enqueue_instant: Instant::now(),
            bytes,
        });
        let limit = self.pending_frame_limit();
        while self.pending_frames.len() > limit {
            self.pending_frames.pop_front();
        }
        seq
    }

    pub fn on_frame_delivered(&mut self, seq: u64) {
        let baseline_rtt = self
            .users
            .values()
            .filter_map(|u| u.delay.rtt_calculator.get_rtt())
            .min()
            .unwrap_or(0);
        if let Some(pos) = self.pending_frames.iter().position(|f| f.seq == seq) {
            if let Some(frame) = self.pending_frames.remove(pos) {
                let elapsed = frame.enqueue_instant.elapsed().as_millis() as u32;
                let queue_delay = elapsed.saturating_sub(baseline_rtt);
                self.last_queue_delay_ms = queue_delay;
                if self.hq_enabled {
                    self.hq_on_queue_delay(queue_delay);
                }
            }
        }
        // Unmatched ack: ignore (do not borrow another frame's delay).
    }

    pub fn on_video_frame_fetched(&mut self) {
        if let Some(frame) = self.pending_frames.front() {
            let seq = frame.seq;
            self.on_frame_delivered(seq);
        }
    }

    pub fn pending_video_bytes(&self) -> u32 {
        self.pending_frames.iter().map(|f| f.bytes).sum()
    }

    // Check if any user is in recording mode
    pub fn record(&self) -> bool {
        self.users.iter().any(|u| u.1.record)
    }

    pub fn set_support_changing_quality(&mut self, video_service_name: &str, support: bool) {
        if let Some(display) = self.displays.get_mut(video_service_name) {
            display.support_changing_quality = support;
        }
    }

    // Check if variable bitrate encoding is supported and enabled
    pub fn in_vbr_state(&self) -> bool {
        if self.hq_enabled {
            if let Some(rate) = &self.hq_rate {
                return rate.mode != RateControlMode::FixedBitrate
                    && self.displays.iter().all(|e| e.1.support_changing_quality);
            }
        }
        self.abr_config && self.displays.iter().all(|e| e.1.support_changing_quality)
    }
}

// User session management
impl VideoQoS {
    // Initialize new user session
    pub fn on_connection_open(&mut self, id: i32) {
        self.users.insert(id, UserData::default());
        self.abr_config = Config::get_option("enable-abr") != "N";
        self.new_user_instant = Instant::now();
    }

    // Clean up user session
    pub fn on_connection_close(&mut self, id: i32) {
        self.users.remove(&id);
        if self.users.is_empty() {
            *self = Default::default();
        } else {
            self.refresh_hq_from_users();
        }
    }

    pub fn user_custom_fps(&mut self, id: i32, fps: u32) {
        if fps < MIN_FPS || fps > MAX_FPS {
            return;
        }
        if let Some(user) = self.users.get_mut(&id) {
            user.custom_fps = Some(fps);
        }
    }

    pub fn user_auto_adjust_fps(&mut self, id: i32, fps: u32) {
        if fps < MIN_FPS || fps > MAX_FPS {
            return;
        }
        if let Some(user) = self.users.get_mut(&id) {
            user.auto_adjust_fps = Some(fps);
        }
    }

    /// Apply an HQ VideoProfile from OptionMessage negotiation.
    ///
    /// Also stores a legacy `user.quality` Custom ratio so that if HQ is later
    /// disabled (all HQ peers disconnect / `refresh_hq_from_users` clears
    /// `hq_enabled`), `latest_quality()` can still fall back to a sensible ratio
    /// derived from the last HQ target bitrate.
    pub fn user_video_profile(&mut self, id: i32, profile: VideoProfile) {
        let rate = profile.rate.clone().clamp();
        if let Some(user) = self.users.get_mut(&id) {
            user.rate_config = Some(rate.clone());
            user.profile_type = Some(profile.profile_type);
            user.custom_fps = Some(rate.target_fps);
            let ratio = rate.to_legacy_ratio();
            user.quality = Some((hbb_common::get_time(), Quality::Custom(ratio)));
        }
        self.refresh_hq_from_users();
        log::info!(
            "HQ video profile applied for user {}: type={:?}, mode={:?}, target={}kbps, fps={}",
            id,
            profile.profile_type,
            rate.mode,
            rate.target_kbps,
            rate.target_fps
        );
    }

    /// Recompute session-level HQ state from the newest per-user profile.
    /// When no user still has a rate_config, HQ is disabled and the legacy
    /// delay/ratio path takes over again (using each user's stored `quality`).
    fn refresh_hq_from_users(&mut self) {
        let latest = self
            .users
            .values()
            .filter_map(|u| {
                u.rate_config.as_ref().map(|r| {
                    (
                        u.quality.map(|q| q.0).unwrap_or(0),
                        r.clone(),
                        u.profile_type.unwrap_or(VideoProfileType::Custom),
                    )
                })
            })
            .max_by_key(|(t, _, _)| *t);
        if let Some((_, rate, profile)) = latest {
            self.hq_enabled = true;
            self.hq_rate = Some(rate.clone());
            self.hq_profile = Some(profile);
            self.hq_target_kbps = rate.target_kbps;
            if self.hq_current_kbps == 0 {
                self.hq_current_kbps = rate.target_kbps;
            } else {
                self.hq_current_kbps = self.hq_current_kbps.clamp(rate.min_kbps, rate.max_kbps);
            }
            self.fps = rate.target_fps.clamp(MIN_FPS, MAX_FPS);
            self.ratio = rate.to_legacy_ratio();
        } else {
            self.hq_enabled = false;
            self.hq_rate = None;
            self.hq_profile = None;
        }
    }

    fn hq_on_queue_delay(&mut self, queue_delay: u32) {
        let Some(rate) = self.hq_rate.clone() else {
            return;
        };
        if rate.mode == RateControlMode::FixedBitrate {
            if queue_delay > rate.max_queue_ms {
                self.drop_old_frames = true;
                self.need_keyframe = true;
                self.hq_state = QosState::Emergency;
                log::info!(
                    "HQ FixedBitrate emergency: queue_delay={}ms > max_queue_ms={}",
                    queue_delay,
                    rate.max_queue_ms
                );
            }
            return;
        }

        let frozen = self
            .hq_freeze_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false);

        let new_state = if queue_delay > HQ_QUEUE_EMERGENCY {
            QosState::Emergency
        } else if queue_delay > rate.max_queue_ms.max(HQ_QUEUE_HOLD) {
            QosState::Congested
        } else if queue_delay > HQ_QUEUE_PROBE {
            QosState::Hold
        } else if !frozen {
            QosState::ProbeUp
        } else {
            QosState::Stable
        };

        if new_state != self.hq_state {
            log::info!(
                "HQ QoS state {:?} -> {:?}, queue_delay={}ms, kbps={}",
                self.hq_state,
                new_state,
                queue_delay,
                self.hq_current_kbps
            );
            self.hq_state = new_state;
        }

        match self.hq_state {
            QosState::ProbeUp => {
                self.hq_good_samples += 1;
                if self.hq_good_samples >= 3 && !frozen {
                    let inc = (self.hq_current_kbps * rate.recovery_increase_percent / 100).max(1);
                    self.hq_current_kbps = (self.hq_current_kbps + inc).min(rate.max_kbps);
                    self.hq_good_samples = 0;
                    self.apply_hq_degradation_or_recovery(false, &rate);
                }
            }
            QosState::Hold => {
                self.hq_good_samples = 0;
            }
            QosState::Congested => {
                self.hq_good_samples = 0;
                let dec = (self.hq_current_kbps * rate.congestion_reduce_percent / 100).max(1);
                self.hq_current_kbps = self.hq_current_kbps.saturating_sub(dec).max(rate.min_kbps);
                self.hq_freeze_until =
                    Some(Instant::now() + Duration::from_millis(rate.recovery_freeze_ms as u64));
                self.apply_hq_degradation_or_recovery(true, &rate);
            }
            QosState::Emergency => {
                self.hq_good_samples = 0;
                let dec = (self.hq_current_kbps * 30 / 100).max(1);
                self.hq_current_kbps = self.hq_current_kbps.saturating_sub(dec).max(rate.min_kbps);
                self.drop_old_frames = true;
                self.need_keyframe = true;
                self.hq_freeze_until =
                    Some(Instant::now() + Duration::from_millis(rate.recovery_freeze_ms as u64));
                self.apply_hq_degradation_or_recovery(true, &rate);
            }
            QosState::Stable => {
                self.hq_good_samples = 0;
            }
        }
        // Keep self.ratio in sync for non-HQ readers / logging; the HQ
        // ratio() getter recomputes from effective_kbps() and ignores this field.
        self.ratio =
            (self.hq_current_kbps as f32 / BASE_1080P_KBPS).clamp(BR_MIN_HIGH_RESOLUTION, BR_MAX);
    }

    fn apply_hq_degradation_or_recovery(&mut self, congested: bool, rate: &VideoRateConfig) {
        let profile = self.hq_profile.unwrap_or(VideoProfileType::Custom);
        match profile {
            VideoProfileType::OfficeClear | VideoProfileType::Custom => {
                if congested {
                    if self.fps > rate.min_fps {
                        self.fps = (self.fps.saturating_sub(2)).max(rate.min_fps);
                    }
                } else if self.fps < rate.target_fps {
                    self.fps = (self.fps + 1).min(rate.target_fps);
                }
            }
            VideoProfileType::MotionSmooth => {
                if congested {
                    if self.hq_state == QosState::Emergency && self.fps > rate.min_fps {
                        self.fps = (self.fps.saturating_sub(5)).max(rate.min_fps);
                    }
                } else if self.fps < rate.target_fps {
                    self.fps = (self.fps + 2).min(rate.max_fps);
                }
            }
            VideoProfileType::TcpStable => {
                if congested {
                    if self.fps > rate.min_fps {
                        self.fps = (self.fps.saturating_sub(3)).max(rate.min_fps);
                    }
                } else if self.fps < rate.target_fps {
                    self.fps = (self.fps + 1).min(rate.target_fps);
                }
            }
        }
    }

    pub fn user_image_quality(&mut self, id: i32, image_quality: i32) {
        let convert_quality = |q: i32| -> Quality {
            if q == ImageQuality::Balanced.value() {
                Quality::Balanced
            } else if q == ImageQuality::Low.value() {
                Quality::Low
            } else if q == ImageQuality::Best.value() {
                Quality::Best
            } else {
                let b = ((q >> 8 & 0xFFF) * 2) as f32 / 100.0;
                Quality::Custom(b.clamp(BR_MIN, BR_MAX))
            }
        };

        let quality = Some((hbb_common::get_time(), convert_quality(image_quality)));
        if let Some(user) = self.users.get_mut(&id) {
            user.quality = quality;
            // update ratio directly (legacy path only)
            if !self.hq_enabled {
                self.ratio = self.latest_quality().ratio();
            }
        }
    }

    pub fn user_record(&mut self, id: i32, v: bool) {
        if let Some(user) = self.users.get_mut(&id) {
            user.record = v;
        }
    }

    pub fn user_network_delay(&mut self, id: i32, delay: u32) {
        if self.hq_enabled {
            // Still record delay/RTT for queue_delay baseline.
            if let Some(user) = self.users.get_mut(&id) {
                user.delay.add_delay(delay.max(10));
            }
            return;
        }
        let highest_fps = self.highest_fps();
        let target_ratio = self.latest_quality().ratio();

        // For bad network, small fps means quick reaction and high quality
        let (min_fps, normal_fps) = if target_ratio >= BR_BEST {
            (8, 16)
        } else if target_ratio >= BR_BALANCED {
            (10, 20)
        } else {
            (12, 24)
        };

        // Calculate minimum acceptable delay-fps product
        let dividend_ms = DELAY_THRESHOLD_150MS * min_fps;

        let mut adjust_ratio = false;
        if let Some(user) = self.users.get_mut(&id) {
            let delay = delay.max(10);
            let old_avg_delay = user.delay.avg_delay();
            user.delay.add_delay(delay);
            let mut avg_delay = user.delay.avg_delay();
            avg_delay = avg_delay.max(10);
            let mut fps = self.fps;

            // Adaptive FPS adjustment based on network delay:
            if avg_delay < 50 {
                user.delay.quick_increase_fps_count += 1;
                let mut step = if fps < normal_fps { 1 } else { 0 };
                if user.delay.quick_increase_fps_count >= 3 {
                    // After 3 consecutive good samples, increase more aggressively
                    user.delay.quick_increase_fps_count = 0;
                    step = 5;
                }
                fps = min_fps.max(fps + step);
            } else if avg_delay < 100 {
                let step = if avg_delay < old_avg_delay {
                    if fps < normal_fps {
                        1
                    } else {
                        0
                    }
                } else {
                    0
                };
                fps = min_fps.max(fps + step);
            } else if avg_delay < DELAY_THRESHOLD_150MS {
                fps = min_fps.max(fps);
            } else {
                let devide_fps = ((fps as f32) / (avg_delay as f32 / DELAY_THRESHOLD_150MS as f32))
                    .ceil() as u32;
                if avg_delay < 200 {
                    fps = min_fps.max(devide_fps);
                } else if avg_delay < 300 {
                    fps = min_fps.min(devide_fps);
                } else if avg_delay < 600 {
                    fps = dividend_ms / avg_delay;
                } else {
                    fps = (dividend_ms / avg_delay).min(devide_fps);
                }
            }

            if avg_delay < DELAY_THRESHOLD_150MS {
                user.delay.increase_fps_count += 1;
            } else {
                user.delay.increase_fps_count = 0;
            }
            if user.delay.increase_fps_count >= 3 {
                // After 3 stable samples, try increasing FPS
                user.delay.increase_fps_count = 0;
                fps += 1;
            }

            // Reset quick increase counter if network condition worsens
            if avg_delay > 50 {
                user.delay.quick_increase_fps_count = 0;
            }

            fps = fps.clamp(MIN_FPS, highest_fps);
            // first network delay message
            adjust_ratio = user.delay.fps.is_none();
            user.delay.fps = Some(fps);
        }
        self.adjust_fps();
        if adjust_ratio && !cfg!(target_os = "linux") {
            //Reduce the possibility of vaapi being created twice
            self.adjust_ratio(false);
        }
    }

    pub fn user_delay_response_elapsed(&mut self, id: i32, elapsed: u128) {
        if let Some(user) = self.users.get_mut(&id) {
            user.delay.response_delayed = elapsed > 2000;
            if user.delay.response_delayed {
                user.delay.add_delay(elapsed as u32);
                self.adjust_fps();
            }
        }
    }
}

// Common adjust functions
impl VideoQoS {
    pub fn new_display(&mut self, video_service_name: String) {
        self.displays
            .insert(video_service_name, DisplayData::default());
    }

    pub fn remove_display(&mut self, video_service_name: &str) {
        self.displays.remove(video_service_name);
    }

    pub fn update_display_data(&mut self, video_service_name: &str, send_counter: usize) {
        if let Some(display) = self.displays.get_mut(video_service_name) {
            display.send_counter += send_counter;
        }
        if self.hq_enabled {
            // HQ path owns fps/bitrate via queue-delay state machine.
            // Still refresh display counters.
            if self.adjust_ratio_instant.elapsed().as_secs() >= ADJUST_RATIO_INTERVAL as u64 {
                self.displays.iter_mut().for_each(|d| {
                    d.1.send_counter = 0;
                });
                self.adjust_ratio_instant = Instant::now();
            }
            return;
        }
        self.adjust_fps();
        let abr_enabled = self.in_vbr_state();
        if abr_enabled {
            if self.adjust_ratio_instant.elapsed().as_secs() >= ADJUST_RATIO_INTERVAL as u64 {
                let dynamic_screen = self
                    .displays
                    .iter()
                    .any(|d| d.1.send_counter >= ADJUST_RATIO_INTERVAL * DYNAMIC_SCREEN_THRESHOLD);
                self.displays.iter_mut().for_each(|d| {
                    d.1.send_counter = 0;
                });
                self.adjust_ratio(dynamic_screen);
            }
        } else {
            self.ratio = self.latest_quality().ratio();
        }
    }

    #[inline]
    fn highest_fps(&self) -> u32 {
        let user_fps = |u: &UserData| {
            let mut fps = u.custom_fps.unwrap_or(FPS);
            if let Some(auto_adjust_fps) = u.auto_adjust_fps {
                if fps == 0 || auto_adjust_fps < fps {
                    fps = auto_adjust_fps;
                }
            }
            fps
        };

        let fps = self
            .users
            .iter()
            .map(|(_, u)| user_fps(u))
            .filter(|u| *u >= MIN_FPS)
            .min()
            .unwrap_or(FPS);

        fps.clamp(MIN_FPS, MAX_FPS)
    }

    // Get latest quality settings from all users
    pub fn latest_quality(&self) -> Quality {
        self.users
            .iter()
            .map(|(_, u)| u.quality)
            .filter(|q| *q != None)
            .max_by(|a, b| a.unwrap_or_default().0.cmp(&b.unwrap_or_default().0))
            .flatten()
            .unwrap_or((0, Quality::Balanced))
            .1
    }

    // Adjust quality ratio based on network delay and screen changes
    fn adjust_ratio(&mut self, dynamic_screen: bool) {
        if !self.in_vbr_state() {
            return;
        }
        // Get maximum delay from all users
        let max_delay = self.users.iter().map(|u| u.1.delay.avg_delay()).max();
        let Some(max_delay) = max_delay else {
            return;
        };

        let target_quality = self.latest_quality();
        let target_ratio = self.latest_quality().ratio();
        let current_ratio = self.ratio;
        let current_bitrate = self.bitrate();

        // Calculate minimum ratio for high resolution (1Mbps baseline)
        let ratio_1mbps = if current_bitrate > 0 {
            Some((current_ratio * 1000.0 / current_bitrate as f32).max(BR_MIN_HIGH_RESOLUTION))
        } else {
            None
        };

        // Calculate ratio for adding 150kbps bandwidth
        let ratio_add_150kbps = if current_bitrate > 0 {
            Some((current_bitrate + 150) as f32 * current_ratio / current_bitrate as f32)
        } else {
            None
        };

        // Set minimum ratio based on quality mode
        let min = match target_quality {
            Quality::Best => {
                // For Best quality, ensure minimum 1Mbps for high resolution
                let mut min = BR_BEST / 2.5;
                if let Some(ratio_1mbps) = ratio_1mbps {
                    if min > ratio_1mbps {
                        min = ratio_1mbps;
                    }
                }
                min.max(BR_MIN)
            }
            Quality::Balanced => {
                let mut min = (BR_BALANCED / 2.0).min(0.4);
                if let Some(ratio_1mbps) = ratio_1mbps {
                    if min > ratio_1mbps {
                        min = ratio_1mbps;
                    }
                }
                min.max(BR_MIN_HIGH_RESOLUTION)
            }
            Quality::Low => BR_MIN_HIGH_RESOLUTION,
            Quality::Custom(_) => BR_MIN_HIGH_RESOLUTION,
        };
        let max = target_ratio * MAX_BR_MULTIPLE;

        let mut v = current_ratio;

        // Adjust ratio based on network delay thresholds
        if max_delay < 50 {
            if dynamic_screen {
                v = current_ratio * 1.15;
            }
        } else if max_delay < 100 {
            if dynamic_screen {
                v = current_ratio * 1.1;
            }
        } else if max_delay < DELAY_THRESHOLD_150MS {
            if dynamic_screen {
                v = current_ratio * 1.05;
            }
        } else if max_delay < 200 {
            v = current_ratio * 0.95;
        } else if max_delay < 300 {
            v = current_ratio * 0.9;
        } else if max_delay < 500 {
            v = current_ratio * 0.85;
        } else {
            v = current_ratio * 0.8;
        }

        // Limit quality increase rate for better stability
        if let Some(ratio_add_150kbps) = ratio_add_150kbps {
            if v > ratio_add_150kbps
                && ratio_add_150kbps > current_ratio
                && current_ratio >= BR_SPEED
            {
                v = ratio_add_150kbps;
            }
        }

        self.ratio = v.clamp(min, max);
        self.adjust_ratio_instant = Instant::now();
    }

    // Adjust fps based on network delay and user response time
    fn adjust_fps(&mut self) {
        let highest_fps = self.highest_fps();
        // Get minimum fps from all users
        let mut fps = self
            .users
            .iter()
            .map(|u| u.1.delay.fps.unwrap_or(INIT_FPS))
            .min()
            .unwrap_or(INIT_FPS);

        if self.users.iter().any(|u| u.1.delay.response_delayed) {
            if fps > MIN_FPS + 1 {
                fps = MIN_FPS + 1;
            }
        }

        // For new connections (within 1 second), cap fps to INIT_FPS to ensure stability
        if self.new_user_instant.elapsed().as_secs() < 1 {
            if fps > INIT_FPS {
                fps = INIT_FPS;
            }
        }

        // Ensure fps stays within valid range
        self.fps = fps.clamp(MIN_FPS, highest_fps);
    }
}

#[derive(Default, Debug, Clone)]
struct RttCalculator {
    min_rtt: Option<u32>,        // Historical minimum RTT ever observed
    window_min_rtt: Option<u32>, // Minimum RTT within last 60 samples
    smoothed_rtt: Option<u32>,   // Smoothed RTT estimation
    samples: VecDeque<u32>,      // Last 60 RTT samples
}

impl RttCalculator {
    const WINDOW_SAMPLES: usize = 60; // Keep last 60 samples
    const MIN_SAMPLES: usize = 10; // Require at least 10 samples
    const ALPHA: f32 = 0.5; // Smoothing factor for weighted average

    /// Update RTT estimates with a new sample
    pub fn update(&mut self, delay: u32) {
        // 1. Update historical minimum RTT
        match self.min_rtt {
            Some(min_rtt) if delay < min_rtt => self.min_rtt = Some(delay),
            None => self.min_rtt = Some(delay),
            _ => {}
        }

        // 2. Update sample window
        if self.samples.len() >= Self::WINDOW_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(delay);

        // 3. Calculate minimum RTT within the window
        self.window_min_rtt = self.samples.iter().min().copied();

        // 4. Calculate smoothed RTT
        // Use weighted average if we have enough samples
        if self.samples.len() >= Self::WINDOW_SAMPLES {
            if let (Some(min), Some(window_min)) = (self.min_rtt, self.window_min_rtt) {
                // Weighted average of historical minimum and window minimum
                let new_srtt =
                    ((1.0 - Self::ALPHA) * min as f32 + Self::ALPHA * window_min as f32) as u32;
                self.smoothed_rtt = Some(new_srtt);
            }
        }
    }

    /// Get current RTT estimate
    /// Returns None if no valid estimation is available
    pub fn get_rtt(&self) -> Option<u32> {
        if let Some(rtt) = self.smoothed_rtt {
            return Some(rtt);
        }
        if self.samples.len() >= Self::MIN_SAMPLES {
            if let Some(rtt) = self.min_rtt {
                return Some(rtt);
            }
        }
        None
    }
}

#[cfg(test)]
mod hq_tests {
    use super::*;
    use hbb_common::video_profile::{
        RateControlMode, ResolutionTier, VideoProfile, VideoProfileType,
    };

    #[test]
    fn hq_profile_sets_current_kbps_to_target() {
        let mut qos = VideoQoS::default();
        qos.on_connection_open(1);
        let profile = VideoProfile::office_clear(ResolutionTier::P1080);
        qos.user_video_profile(1, profile.clone());
        assert!(qos.hq_enabled());
        assert_eq!(qos.current_kbps(), profile.rate.target_kbps);
        assert_eq!(qos.bitrate(), profile.rate.target_kbps);
    }

    #[test]
    fn congested_queue_delay_reduces_current_kbps() {
        let mut qos = VideoQoS::default();
        qos.on_connection_open(1);
        let profile = VideoProfile::office_clear(ResolutionTier::P1080);
        let target = profile.rate.target_kbps;
        qos.user_video_profile(1, profile);
        assert_eq!(qos.current_kbps(), target);

        // Simulate congested queue delay (> hold / max_queue_ms).
        qos.hq_on_queue_delay(200);
        assert!(qos.current_kbps() < target);
        assert_eq!(qos.hq_state(), QosState::Congested);

        // The live bitrate exposed to check_qos must track hq_current_kbps.
        let mut rate = qos.hq_rate_config().cloned().unwrap();
        rate.target_kbps = qos.current_kbps().clamp(rate.min_kbps, rate.max_kbps);
        assert_eq!(rate.target_kbps, qos.current_kbps());
        assert!(rate.target_kbps < target);
    }

    #[test]
    fn clear_pending_frames_does_not_drive_state_machine() {
        let mut qos = VideoQoS::default();
        qos.on_connection_open(1);
        qos.user_video_profile(1, VideoProfile::tcp_stable(ResolutionTier::P1080));
        let before = qos.hq_state();
        let kbps_before = qos.current_kbps();
        qos.on_frame_enqueue(1000);
        qos.on_frame_enqueue(1000);
        qos.clear_pending_frames();
        assert_eq!(qos.pending_video_bytes(), 0);
        assert_eq!(qos.hq_state(), before);
        assert_eq!(qos.current_kbps(), kbps_before);
    }

    #[test]
    fn unmatched_frame_ack_does_not_pop_front() {
        let mut qos = VideoQoS::default();
        let seq = qos.on_frame_enqueue(500);
        qos.on_frame_delivered(seq.wrapping_add(999));
        assert_eq!(qos.pending_video_bytes(), 500);
    }

    #[test]
    fn tcp_stable_recovers_fps_after_congestion() {
        let mut qos = VideoQoS::default();
        qos.on_connection_open(1);
        let mut profile = VideoProfile::tcp_stable(ResolutionTier::P1080);
        profile.rate.mode = RateControlMode::Auto;
        let target_fps = profile.rate.target_fps;
        qos.user_video_profile(1, profile);
        assert_eq!(qos.fps(), target_fps);

        qos.hq_on_queue_delay(200); // Congested → FPS drops
        let fps_after_drop = qos.fps();
        assert!(fps_after_drop < target_fps);

        // Clear recovery freeze so ProbeUp can raise FPS again.
        qos.hq_freeze_until = None;
        for _ in 0..12 {
            qos.hq_on_queue_delay(30);
        }
        assert!(qos.fps() > fps_after_drop);
    }

    #[test]
    fn named_preset_uses_resolution_tier_on_server() {
        let p4k = VideoProfile::for_type(VideoProfileType::OfficeClear, 3840, 2160);
        assert_eq!(p4k.rate.target_kbps, 40_000);
        let p1080 = VideoProfile::for_type(VideoProfileType::OfficeClear, 1920, 1080);
        assert_eq!(p1080.rate.target_kbps, 16_000);
        assert!(p4k.rate.target_kbps > p1080.rate.target_kbps);
    }
}
