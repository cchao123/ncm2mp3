import { useCallback, useEffect, useRef, useState } from "react";
import { api, AVAILABILITY_LABEL, fmtDuration, Track } from "../api";
import type { Player } from "../player";

const PAGE = 30;

/**
 * 搜索页（ADR-0007）。
 *
 * 交互刻意做成「一行一个动作」：每行右侧直接是试听和下载按钮，不做勾选和批量选择。
 * 搜索的典型用法是找到某几首就下，批量选择反而多一道手续。
 *
 * 可用性必须在**点击之前**标出来：搜索必然会搜到账号权限之外的曲目，
 * 下不了的置灰并注明原因。按 ADR-0001，对这些曲目不做任何绕过尝试。
 */
export default function Search({
  onQueued,
  player,
}: {
  onQueued: () => void;
  player: Player;
}) {
  const [kw, setKw] = useState("");
  const [tracks, setTracks] = useState<Track[]>([]);
  const [total, setTotal] = useState(0);
  const [have, setHave] = useState<Set<number>>(new Set());
  const [queued, setQueued] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);
  const [error, setError] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const markDownloaded = useCallback(async (list: Track[]) => {
    if (!list.length) return;
    try {
      const ids = await api.filterDownloaded(list.map((t) => t.id));
      setHave((prev) => new Set([...prev, ...ids]));
    } catch {
      /* 「已下载」只是提示，查询失败不该打断搜索 */
    }
  }, []);

  async function runSearch(reset: boolean) {
    const q = kw.trim();
    if (!q || loading) return;

    setLoading(true);
    setError("");
    if (reset) {
      setTracks([]);
      setHave(new Set());
      setQueued(new Set());
    }

    try {
      const offset = reset ? 0 : tracks.length;
      const r = await api.searchSongs(q, PAGE, offset);
      setTracks(reset ? r.tracks : [...tracks, ...r.tracks]);
      setTotal(r.total);
      setSearched(true);
      markDownloaded(r.tracks);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function download(t: Track) {
    if (t.availability !== "downloadable") return;
    try {
      await api.enqueue([t]);
      setQueued((prev) => new Set(prev).add(t.id));
      onQueued();
    } catch (e) {
      setError(String(e));
    }
  }

  const blocked = tracks.filter((t) => t.availability !== "downloadable").length;

  return (
    <>
      <div className="topbar">
        <h1>搜索</h1>
        <input
          ref={inputRef}
          className="search-input"
          placeholder="歌名、歌手或专辑，回车搜索"
          value={kw}
          onChange={(e) => setKw(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && runSearch(true)}
        />
        <button onClick={() => runSearch(true)} disabled={loading || !kw.trim()}>
          {loading && !tracks.length ? "搜索中…" : "搜索"}
        </button>
        <div className="spacer" />
        {tracks.length > 0 && (
          <span className="faint">
            共 {total} 条，已显示 {tracks.length}
            {blocked > 0 && ` · ${blocked} 首无下载权限`}
          </span>
        )}
      </div>

      <div className="content">
        {error && <div className="notice err">{error}</div>}

        {!searched && !loading && (
          <div className="muted" style={{ marginTop: 44, textAlign: "center" }}>
            搜索你想下载的歌，先试听，再下载。
            <div className="faint" style={{ marginTop: 8 }}>
              只能下载你账号权限内的曲目，其余会标注原因并置灰。
            </div>
          </div>
        )}

        {searched && !tracks.length && !loading && (
          <div className="muted">没有搜到结果。</div>
        )}

        {tracks.map((t) => {
          const downloaded = have.has(t.id);
          const inQueue = queued.has(t.id);
          const usable = t.availability === "downloadable";
          const label = AVAILABILITY_LABEL[t.availability];
          const isCurrent = player.track?.id === t.id;

          return (
            <div className={`row ${!usable ? "row-disabled" : ""}`} key={t.id}>
              <button
                className="icon-btn"
                title={isCurrent && player.playing ? "暂停" : "试听"}
                onClick={() => player.toggle(t)}
              >
                {isCurrent && player.loading
                  ? "…"
                  : isCurrent && player.playing
                  ? "❚❚"
                  : "▶"}
              </button>

              <span className={`title ${isCurrent ? "playing" : ""}`}>{t.title}</span>
              <span className="sub">{t.artists}</span>
              <span className="sub">{t.album}</span>

              {downloaded && <span className="pill ok">已下载</span>}
              {!downloaded && inQueue && <span className="pill pending">已加入队列</span>}
              {label && <span className="pill err">{label}</span>}

              <span className="tail">{fmtDuration(t.duration_ms)}</span>

              <button
                className="primary"
                style={{ padding: "5px 13px", fontSize: 12.5 }}
                disabled={!usable || downloaded || inQueue}
                onClick={() => download(t)}
              >
                {downloaded ? "已下载" : inQueue ? "已加入" : "下载"}
              </button>
            </div>
          );
        })}

        {tracks.length > 0 && tracks.length < total && (
          <div style={{ textAlign: "center", marginTop: 18 }}>
            <button onClick={() => runSearch(false)} disabled={loading}>
              {loading ? "加载中…" : "加载更多"}
            </button>
          </div>
        )}
      </div>
    </>
  );
}
