/// Thin wrapper around the YouTube IFrame Player API. Owns the iframe
/// lifecycle and normalizes the player's quirky state machine into a
/// promise-based API: `ready`, then `play / pause / seekTo / loadVideo`.
///
/// The IFrame Player API ships its globals lazily via a script tag and a
/// global `onYouTubeIframeAPIReady` callback. We deduplicate the load so
/// multiple watch rooms on the same page don't double-inject.

declare global {
  interface Window {
    YT?: YTNamespace;
    onYouTubeIframeAPIReady?: () => void;
  }
}

interface YTNamespace {
  Player: new (
    element: HTMLElement | string,
    options: YTPlayerOptions,
  ) => YTPlayerInstance;
  PlayerState: {
    UNSTARTED: -1;
    ENDED: 0;
    PLAYING: 1;
    PAUSED: 2;
    BUFFERING: 3;
    CUED: 5;
  };
}

interface YTPlayerOptions {
  width?: string | number;
  height?: string | number;
  videoId?: string;
  playerVars?: Record<string, unknown>;
  events?: {
    onReady?: (event: { target: YTPlayerInstance }) => void;
    onStateChange?: (event: { data: number; target: YTPlayerInstance }) => void;
    onError?: (event: { data: number }) => void;
  };
}

interface YTPlayerInstance {
  playVideo(): void;
  pauseVideo(): void;
  seekTo(seconds: number, allowSeekAhead: boolean): void;
  getCurrentTime(): number;
  getDuration(): number;
  getPlayerState(): number;
  loadVideoById(opts: { videoId: string; startSeconds?: number }): void;
  cueVideoById(opts: { videoId: string; startSeconds?: number }): void;
  destroy(): void;
  setPlaybackRate(rate: number): void;
}

let apiLoad: Promise<YTNamespace> | null = null;

function loadApi(): Promise<YTNamespace> {
  if (apiLoad) return apiLoad;
  apiLoad = new Promise<YTNamespace>((resolve, reject) => {
    if (typeof window === 'undefined') {
      reject(new Error('YouTube player is browser-only'));
      return;
    }
    if (window.YT?.Player) {
      resolve(window.YT);
      return;
    }
    // Chain any existing callback so we don't clobber another loader.
    const previous = window.onYouTubeIframeAPIReady;
    window.onYouTubeIframeAPIReady = () => {
      if (previous) previous();
      if (window.YT?.Player) {
        resolve(window.YT);
      } else {
        reject(new Error('YT API loaded without Player constructor'));
      }
    };
    const tag = document.createElement('script');
    tag.src = 'https://www.youtube.com/iframe_api';
    tag.async = true;
    tag.onerror = () => reject(new Error('Failed to load YouTube IFrame API'));
    document.head.appendChild(tag);
  });
  return apiLoad;
}

export type PlayerEvent =
  | { kind: 'play' }
  | { kind: 'pause' }
  | { kind: 'ended' }
  | { kind: 'seek'; position_ms: number };

export interface YouTubePlayer {
  ready: Promise<void>;
  play(): void;
  pause(): void;
  seekTo(positionMs: number): void;
  loadVideo(videoId: string, startMs?: number): void;
  cueVideo(videoId: string, startMs?: number): void;
  getPosition(): number;
  getDuration(): number;
  setRate(rate: number): void;
  on(handler: (e: PlayerEvent) => void): () => void;
  destroy(): void;
}

export function createYouTubePlayer(element: HTMLElement): YouTubePlayer {
  let player: YTPlayerInstance | null = null;
  const handlers: Array<(e: PlayerEvent) => void> = [];
  let lastState = -1;
  let lastPosition = 0;

  const ready = loadApi().then(
    (YT) =>
      new Promise<void>((resolve) => {
        player = new YT.Player(element, {
          width: '100%',
          height: '100%',
          playerVars: {
            // Hide YouTube chrome where possible — the room's controls drive
            // playback. Followers shouldn't be able to play/pause via the
            // iframe directly since their events would be ignored anyway.
            controls: 0,
            disablekb: 1,
            modestbranding: 1,
            rel: 0,
            playsinline: 1,
          },
          events: {
            onReady: () => resolve(),
            onStateChange: (event) => {
              lastState = event.data;
              const t = player?.getCurrentTime() ?? 0;
              switch (event.data) {
                case YT.PlayerState.PLAYING:
                  // A seek-while-playing fires PLAYING again with a jump in
                  // current time; emit a seek if the jump exceeds 1s.
                  if (Math.abs(t - lastPosition) > 1) {
                    emit({ kind: 'seek', position_ms: Math.round(t * 1000) });
                  }
                  emit({ kind: 'play' });
                  break;
                case YT.PlayerState.PAUSED:
                  if (Math.abs(t - lastPosition) > 1) {
                    emit({ kind: 'seek', position_ms: Math.round(t * 1000) });
                  }
                  emit({ kind: 'pause' });
                  break;
                case YT.PlayerState.ENDED:
                  emit({ kind: 'ended' });
                  break;
              }
              lastPosition = t;
            },
          },
        });
      }),
  );

  function emit(event: PlayerEvent): void {
    for (const h of handlers) h(event);
  }

  return {
    ready,
    play() {
      player?.playVideo();
    },
    pause() {
      player?.pauseVideo();
    },
    seekTo(positionMs: number) {
      const seconds = Math.max(0, positionMs) / 1000;
      lastPosition = seconds;
      player?.seekTo(seconds, true);
    },
    loadVideo(videoId: string, startMs = 0) {
      player?.loadVideoById({ videoId, startSeconds: startMs / 1000 });
    },
    cueVideo(videoId: string, startMs = 0) {
      player?.cueVideoById({ videoId, startSeconds: startMs / 1000 });
    },
    getPosition(): number {
      return Math.round((player?.getCurrentTime() ?? 0) * 1000);
    },
    getDuration(): number {
      return Math.round((player?.getDuration() ?? 0) * 1000);
    },
    setRate(rate: number) {
      player?.setPlaybackRate(rate);
    },
    on(handler: (e: PlayerEvent) => void): () => void {
      handlers.push(handler);
      return () => {
        const i = handlers.indexOf(handler);
        if (i >= 0) handlers.splice(i, 1);
      };
    },
    destroy() {
      try {
        player?.destroy();
      } catch {
        /* ignore — player may have been removed from DOM already */
      }
      player = null;
      handlers.length = 0;
    },
  };
}
