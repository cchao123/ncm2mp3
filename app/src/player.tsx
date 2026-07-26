import { useCallback, useEffect, useRef, useState } from "react";
import { api, fmtDuration, Track } from "./api";

/**
 * 试听播放器。
 *
 * 直接把直链交给 <audio> 流式播放，不预先下载整首——试听只是「要不要下载」
 * 的判断依据，等下载完再听就失去意义了。
 *
 * 播放器状态提在 App 层：切换页面时播放不该中断。
 */
export function usePlayer() {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [track, setTrack] = useState<Track | null>(null);
  const [playing, setPlaying] = useState(false);
  const [loading, setLoading] = useState(false);
  const [pos, setPos] = useState(0);
  const [dur, setDur] = useState(0);
  const [error, setError] = useState("");

  if (!audioRef.current && typeof Audio !== "undefined") {
    audioRef.current = new Audio();
  }

  useEffect(() => {
    const a = audioRef.current;
    if (!a) return;
    const onTime = () => setPos(a.currentTime);
    const onMeta = () => setDur(a.duration || 0);
    const onEnd = () => setPlaying(false);
    const onErr = () =>
      setError("试听失败：音频地址可能已过期，或该曲目不提供在线播放");

    a.addEventListener("timeupdate", onTime);
    a.addEventListener("loadedmetadata", onMeta);
    a.addEventListener("ended", onEnd);
    a.addEventListener("error", onErr);
    return () => {
      a.removeEventListener("timeupdate", onTime);
      a.removeEventListener("loadedmetadata", onMeta);
      a.removeEventListener("ended", onEnd);
      a.removeEventListener("error", onErr);
    };
  }, []);

  const toggle = useCallback(
    async (t: Track) => {
      const a = audioRef.current;
      if (!a) return;
      setError("");

      // 点当前正在播的这首 → 暂停 / 继续
      if (track?.id === t.id) {
        if (a.paused) {
          await a.play().catch(() => {});
          setPlaying(true);
        } else {
          a.pause();
          setPlaying(false);
        }
        return;
      }

      setLoading(true);
      setTrack(t);
      setPos(0);
      setDur(0);
      try {
        const url = await api.previewUrl(t.id);
        a.src = url;
        await a.play();
        setPlaying(true);
      } catch (e) {
        setError(String(e));
        setPlaying(false);
        setTrack(null);
      } finally {
        setLoading(false);
      }
    },
    [track]
  );

  const stop = useCallback(() => {
    const a = audioRef.current;
    if (a) {
      a.pause();
      a.src = "";
    }
    setTrack(null);
    setPlaying(false);
    setPos(0);
    setDur(0);
    setError("");
  }, []);

  const seek = useCallback((sec: number) => {
    const a = audioRef.current;
    if (a && Number.isFinite(sec)) a.currentTime = sec;
  }, []);

  return { track, playing, loading, pos, dur, error, toggle, stop, seek };
}

export type Player = ReturnType<typeof usePlayer>;

export function PlayerBar({ player }: { player: Player }) {
  const { track, playing, loading, pos, dur, error, toggle, stop, seek } = player;
  if (!track) return null;

  return (
    <div className="player-bar">
      <button
        className="play-btn"
        onClick={() => toggle(track)}
        title={playing ? "暂停" : "播放"}
      >
        {loading ? "…" : playing ? "❚❚" : "▶"}
      </button>

      <div className="player-meta">
        <div className="player-title">{track.title}</div>
        <div className="faint">{track.artists}</div>
      </div>

      <span className="tail">{fmtDuration(pos * 1000)}</span>
      <input
        type="range"
        className="seek"
        min={0}
        max={dur || 0}
        step={0.5}
        value={Math.min(pos, dur || 0)}
        onChange={(e) => seek(Number(e.target.value))}
        disabled={!dur}
      />
      <span className="tail">{fmtDuration(dur * 1000)}</span>

      {error && <span className="player-err">{error}</span>}

      <button onClick={stop} title="关闭试听">
        ✕
      </button>
    </div>
  );
}
