import type { YouTubePlayer } from './youtube-player';
import type { WatchPlayback } from '$stores/watch';

/// Reconciles a local YouTubePlayer against the server's authoritative
/// playback state. Two pathways:
///
/// 1. Transitions (`apply`): a `watch_playback` or `watch_state` message
///    arrived. Hard-set the player to the projected position, then play or
///    pause to match.
///
/// 2. Pulses (`reconcile`): a periodic `watch_sync_pulse` arrived. Compare
///    local clock to server's projected position and either:
///       - ignore (<200ms drift — within noise)
///       - rate-correct (200-500ms — speed up/slow down by 1%)
///       - hard seek (>500ms — visible jump, but at least it's consistent)
///
/// All times are milliseconds. The wall-clock offset (`Date.now() vs server's
/// `server_ts`) is treated as ≈0 — we don't NTP-correct, since both sides are
/// browser clocks and most viewers are within 100ms. The drift band hides the
/// rest.
export interface SyncController {
  /// Apply a state transition (play, pause, seek, or full snapshot).
  apply(playback: WatchPlayback, isLeader: boolean): void;
  /// Apply a periodic pulse — soft-corrects drift while playing.
  reconcile(playback: WatchPlayback, isLeader: boolean): void;
  /// Stop any in-flight rate corrections.
  stop(): void;
}

const DRIFT_IGNORE_MS = 200;
const DRIFT_HARD_SEEK_MS = 500;
const RATE_CORRECTION_DURATION_MS = 4_000;
// Bound on `Date.now() - server_ts` we trust for projection. Negative means
// our wall clock is behind the server's; large positive likely means a
// suspend/resume or a stale message that sat in a queue. In either case the
// drift band logic silently breaks, so we fall back to the raw server
// position and let the next pulse re-anchor us.
const PROJECTION_MAX_ELAPSED_MS = 10_000;

export function createSyncController(player: YouTubePlayer): SyncController {
  let lastVideoId: string | null = null;
  let rateRevertTimer: ReturnType<typeof setTimeout> | null = null;

  function projectedServerPosition(playback: WatchPlayback): number {
    if (playback.paused) return playback.position_ms;
    const elapsed = Date.now() - playback.server_ts;
    if (elapsed < 0 || elapsed > PROJECTION_MAX_ELAPSED_MS) {
      return playback.position_ms;
    }
    return playback.position_ms + elapsed * playback.rate;
  }

  function clearRateRevert(): void {
    if (rateRevertTimer) {
      clearTimeout(rateRevertTimer);
      rateRevertTimer = null;
    }
  }

  return {
    apply(playback, isLeader) {
      clearRateRevert();
      player.setRate(1);

      // Load a different video if the queue advanced.
      // Returning early after cueVideo/loadVideo prevents the subsequent
      // play/pause from racing with YouTube's still-loading state machine
      // (which silently ignores commands mid-buffer). The next pulse or
      // transition reconciles state once the player is ready.
      if (playback.video_id && playback.video_id !== lastVideoId) {
        lastVideoId = playback.video_id;
        const start = projectedServerPosition(playback);
        if (playback.paused) {
          player.cueVideo(playback.video_id, start);
        } else {
          player.loadVideo(playback.video_id, start);
        }
        return;
      }
      // If the video was cleared (queue emptied), forget the cached id so the
      // next item loads cleanly.
      if (!playback.video_id) {
        lastVideoId = null;
      }

      // The leader's own player drives `apply` (e.g. on initial load) but we
      // don't want to seek the leader's iframe when they themselves just
      // emitted the event — that round-trip would yank their seek bar.
      if (isLeader) {
        if (playback.paused) {
          player.pause();
        } else {
          player.play();
        }
        return;
      }

      const target = projectedServerPosition(playback);
      const local = player.getPosition();
      if (Math.abs(local - target) > DRIFT_IGNORE_MS) {
        player.seekTo(target);
      }
      if (playback.paused) {
        player.pause();
      } else {
        player.play();
      }
    },

    reconcile(playback, isLeader) {
      if (isLeader || playback.paused || !playback.video_id) return;
      const target = projectedServerPosition(playback);
      const local = player.getPosition();
      const drift = local - target;
      const abs = Math.abs(drift);

      if (abs < DRIFT_IGNORE_MS) {
        return;
      }
      if (abs > DRIFT_HARD_SEEK_MS) {
        clearRateRevert();
        player.setRate(1);
        player.seekTo(target);
        return;
      }
      // 200-500ms band: soft-correct via playback rate so the viewer just
      // perceives a slightly faster/slower stretch. drift > 0 = local ahead
      // of server, so slow down (0.99x).
      const correctedRate = drift > 0 ? 0.99 : 1.01;
      player.setRate(correctedRate);
      clearRateRevert();
      rateRevertTimer = setTimeout(() => {
        player.setRate(1);
        rateRevertTimer = null;
      }, RATE_CORRECTION_DURATION_MS);
    },

    stop() {
      clearRateRevert();
      player.setRate(1);
    },
  };
}
